use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crate::tools::fs_common::{check_cancel, resolve_path};
use astrcode_core::{
    AstrError, Result, SideEffectLevel, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolOutputStream, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

#[derive(Default)]
pub struct ShellTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShellArgs {
    command: String,
    #[serde(default)]
    cwd: Option<PathBuf>,
    #[serde(default)]
    shell: Option<String>,
}

struct CommandSpec {
    program: String,
    args: Vec<String>,
}

struct StreamCapture {
    text: String,
    bytes_read: usize,
    truncated: bool,
    limit: usize,
    stream: ToolOutputStream,
}

impl StreamCapture {
    fn new(stream: ToolOutputStream, limit: usize) -> Self {
        Self {
            text: String::new(),
            bytes_read: 0,
            truncated: false,
            limit,
            stream,
        }
    }

    fn push_chunk(&mut self, chunk: &str) -> String {
        self.bytes_read = self.bytes_read.saturating_add(chunk.len());
        if self.truncated || chunk.is_empty() {
            return String::new();
        }

        let remaining = self.limit.saturating_sub(self.text.len());
        if remaining == 0 {
            self.truncated = true;
            return self.append_truncation_notice();
        }

        let take_len = chunk.floor_char_boundary(remaining.min(chunk.len()));
        let visible = &chunk[..take_len];
        self.text.push_str(visible);

        let mut emitted = visible.to_string();
        if take_len < chunk.len() {
            self.truncated = true;
            let notice = self.append_truncation_notice();
            emitted.push_str(&notice);
        }

        emitted
    }

    fn append_truncation_notice(&mut self) -> String {
        let label = match self.stream {
            ToolOutputStream::Stdout => "stdout",
            ToolOutputStream::Stderr => "stderr",
        };
        let notice = format!(
            "\n... [{label} truncated after {} bytes; later output omitted]\n",
            self.limit
        );
        self.text.push_str(&notice);
        notice
    }
}

fn spawn_stream_reader<R: Read + Send + 'static>(
    reader: R,
    stream: ToolOutputStream,
    ctx: ToolContext,
    tool_call_id: String,
    tool_name: String,
    limit: usize,
) -> thread::JoinHandle<std::result::Result<StreamCapture, std::io::Error>> {
    thread::spawn(move || {
        let mut capture = StreamCapture::new(stream, limit);
        let mut reader = reader;
        let mut chunk = [0u8; 4096];
        let mut pending_bytes = Vec::new();
        let mut pending = String::new();

        loop {
            let read = reader.read(&mut chunk)?;
            if read == 0 {
                if !pending_bytes.is_empty() {
                    // A final lossy flush at EOF preserves any incomplete trailing UTF-8 bytes
                    // instead of silently dropping them from the terminal transcript.
                    pending.push_str(&String::from_utf8_lossy(&pending_bytes));
                    pending_bytes.clear();
                }
                if !pending.is_empty() {
                    let visible = capture.push_chunk(&pending);
                    if !visible.is_empty() {
                        match stream {
                            ToolOutputStream::Stdout => {
                                let _ = ctx.emit_stdout(
                                    tool_call_id.clone(),
                                    tool_name.clone(),
                                    visible,
                                );
                            }
                            ToolOutputStream::Stderr => {
                                let _ = ctx.emit_stderr(
                                    tool_call_id.clone(),
                                    tool_name.clone(),
                                    visible,
                                );
                            }
                        }
                    }
                }
                break;
            }

            pending_bytes.extend_from_slice(&chunk[..read]);
            pending.push_str(&drain_decoded_utf8(&mut pending_bytes));
            while let Some(newline_index) = pending.find('\n') {
                let next_chunk = pending[..=newline_index].to_string();
                pending.replace_range(..=newline_index, "");
                let visible = capture.push_chunk(&next_chunk);
                if visible.is_empty() {
                    continue;
                }

                match stream {
                    ToolOutputStream::Stdout => {
                        let _ = ctx.emit_stdout(tool_call_id.clone(), tool_name.clone(), visible);
                    }
                    ToolOutputStream::Stderr => {
                        let _ = ctx.emit_stderr(tool_call_id.clone(), tool_name.clone(), visible);
                    }
                }
            }

            if pending.len() >= 4096 {
                // Extremely long lines should still stream progressively; otherwise a single
                // no-newline command can hold the entire transcript until process exit.
                let visible = capture.push_chunk(&pending);
                pending.clear();
                if visible.is_empty() {
                    continue;
                }

                match stream {
                    ToolOutputStream::Stdout => {
                        let _ = ctx.emit_stdout(tool_call_id.clone(), tool_name.clone(), visible);
                    }
                    ToolOutputStream::Stderr => {
                        let _ = ctx.emit_stderr(tool_call_id.clone(), tool_name.clone(), visible);
                    }
                }
            }
        }

        Ok(capture)
    })
}

fn drain_decoded_utf8(pending_bytes: &mut Vec<u8>) -> String {
    let mut decoded = String::new();

    loop {
        match std::str::from_utf8(pending_bytes) {
            Ok(valid) => {
                decoded.push_str(valid);
                pending_bytes.clear();
                break;
            }
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                if valid_up_to > 0 {
                    let valid = std::str::from_utf8(&pending_bytes[..valid_up_to])
                        .expect("valid UTF-8 prefix should decode");
                    decoded.push_str(valid);
                    pending_bytes.drain(..valid_up_to);
                    continue;
                }

                let Some(invalid_len) = error.error_len() else {
                    // `error_len == None` means the remaining bytes form an incomplete UTF-8
                    // sequence that may become valid once the next read arrives, so keep them.
                    break;
                };

                decoded.push_str(&String::from_utf8_lossy(&pending_bytes[..invalid_len]));
                pending_bytes.drain(..invalid_len);
            }
        }
    }

    decoded
}

fn render_shell_output(stdout: &str, stderr: &str) -> String {
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => stdout.to_string(),
        (true, false) => stderr.to_string(),
        (false, false) => format!("[stdout]\n{stdout}\n\n[stderr]\n{stderr}"),
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "shell".to_string(),
            description:
                "Execute a non-interactive shell command once and return stdout/stderr/exitCode."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "cwd": { "type": "string" },
                    "shell": { "type": "string" }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .tags(["process", "shell"])
            .permission("shell.exec")
            .side_effect(SideEffectLevel::External)
            .prompt(
                ToolPromptMetadata::new(
                    "Run a one-shot shell command when file tools or search tools are not precise enough.",
                    "Use `shell` for non-interactive commands that are easier to express as a single command line than as a dedicated file tool. Keep commands scoped to the workspace, explain risky commands before running them, and prefer read-only inspection before mutation.",
                )
                .caveat("Shell commands can mutate the workspace or external system state, so keep them narrowly scoped.")
                .example("Inspect repository status or run a targeted build/test command.")
                .prompt_tag("shell"),
            )
    }

    async fn execute(
        &self,
        tool_call_id: String,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        check_cancel(ctx.cancel(), "shell")?;
        let args: ShellArgs = serde_json::from_value(args)
            .map_err(|e| AstrError::parse("invalid args for shell tool", e))?;
        if args.command.trim().is_empty() {
            return Err(AstrError::Validation(
                "shell command cannot be empty".to_string(),
            ));
        }

        let spec = command_spec(args.shell.as_deref(), &args.command);
        let started_at = Instant::now();
        let command_text = args.command.clone();
        let shell_program = spec.program.clone();
        let cwd = match args.cwd {
            Some(cwd) => resolve_path(ctx, &cwd)?,
            None => ctx.working_dir().clone(),
        };
        let cwd_text = cwd.to_string_lossy().to_string();

        let mut child = Command::new(&spec.program)
            .args(&spec.args)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| AstrError::io("failed to spawn shell command", e))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AstrError::Internal("failed to capture stdout".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AstrError::Internal("failed to capture stderr".to_string()))?;
        let stream_limit = ctx.max_output_size();
        let stdout_task = spawn_stream_reader(
            stdout,
            ToolOutputStream::Stdout,
            ctx.clone(),
            tool_call_id.clone(),
            "shell".to_string(),
            stream_limit,
        );
        let stderr_task = spawn_stream_reader(
            stderr,
            ToolOutputStream::Stderr,
            ctx.clone(),
            tool_call_id.clone(),
            "shell".to_string(),
            stream_limit,
        );
        let status = loop {
            if ctx.cancel().is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AstrError::Cancelled);
            }

            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(e) => return Err(AstrError::io("failed to wait shell command", e)),
            }

            thread::sleep(Duration::from_millis(25));
        };

        let stdout_capture = stdout_task
            .join()
            .map_err(|_| AstrError::Internal("stdout reader thread panicked".to_string()))?
            .map_err(|e| AstrError::io("failed to read stdout", e))?;
        let stderr_capture = stderr_task
            .join()
            .map_err(|_| AstrError::Internal("stderr reader thread panicked".to_string()))?
            .map_err(|e| AstrError::io("failed to read stderr", e))?;

        let exit_code = status.code().unwrap_or(-1);
        let ok = status.success();
        let output = render_shell_output(&stdout_capture.text, &stderr_capture.text);

        // Truncate if output exceeds max size
        let (output, truncated) = if output.len() > ctx.max_output_size() {
            let truncation_msg = format!(
                "\n... [OUTPUT TRUNCATED: {} bytes total, showing first {} bytes]",
                output.len(),
                ctx.max_output_size()
            );
            // 使用 floor_char_boundary 确保截断点在 UTF-8 char boundary 上，
            // 避免输出含中文/Unicode 且总量接近 max_output_size 时按字节切片 panic
            let truncate_at = output
                .floor_char_boundary(ctx.max_output_size().saturating_sub(truncation_msg.len()));
            let mut truncated_output = output[..truncate_at].to_string();
            truncated_output.push_str(&truncation_msg);
            (truncated_output, true)
        } else {
            (output, false)
        };

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "shell".to_string(),
            ok,
            output,
            error: if ok {
                None
            } else {
                Some(format!("shell command exited with code {}", exit_code))
            },
            metadata: Some(json!({
                "command": command_text,
                "cwd": cwd_text.clone(),
                "shell": shell_program,
                "exitCode": exit_code,
                "streamed": true,
                "stdoutBytes": stdout_capture.bytes_read,
                "stderrBytes": stderr_capture.bytes_read,
                "stdoutTruncated": stdout_capture.truncated,
                "stderrTruncated": stderr_capture.truncated,
                "display": {
                    "kind": "terminal",
                    "command": args.command,
                    "cwd": cwd_text,
                    "shell": spec.program,
                    "exitCode": exit_code,
                },
                "truncated": truncated,
            })),
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated,
        })
    }
}

fn command_spec(shell: Option<&str>, command: &str) -> CommandSpec {
    #[cfg(windows)]
    {
        let program = match shell {
            Some(shell) => shell.to_string(),
            None => default_windows_shell().to_string(),
        };
        CommandSpec {
            program,
            args: vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                command.to_string(),
            ],
        }
    }

    #[cfg(not(windows))]
    {
        let program = shell.unwrap_or("/bin/sh").to_string();
        CommandSpec {
            program,
            args: vec!["-lc".to_string(), command.to_string()],
        }
    }
}

#[cfg(windows)]
fn default_windows_shell() -> &'static str {
    use std::sync::OnceLock;
    static SHELL: OnceLock<&'static str> = OnceLock::new();
    SHELL.get_or_init(|| {
        if std::process::Command::new("pwsh")
            .arg("-NoProfile")
            .arg("-Command")
            .arg("$PSVersionTable.PSVersion")
            .output()
            .is_ok()
        {
            "pwsh"
        } else {
            "powershell"
        }
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io;

    use super::*;
    use crate::test_support::test_tool_context_for;
    use astrcode_core::ToolOutputDelta;
    use tokio::sync::mpsc;

    struct ChunkedReader {
        chunks: VecDeque<Vec<u8>>,
    }

    impl ChunkedReader {
        fn new(chunks: Vec<Vec<u8>>) -> Self {
            Self {
                chunks: VecDeque::from(chunks),
            }
        }
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Some(chunk) = self.chunks.pop_front() else {
                return Ok(0);
            };
            let read_len = chunk.len().min(buf.len());
            buf[..read_len].copy_from_slice(&chunk[..read_len]);
            if read_len < chunk.len() {
                self.chunks.push_front(chunk[read_len..].to_vec());
            }
            Ok(read_len)
        }
    }

    fn collect_output_deltas(
        rx: &mut mpsc::UnboundedReceiver<ToolOutputDelta>,
    ) -> Vec<ToolOutputDelta> {
        let mut deltas = Vec::new();
        while let Ok(delta) = rx.try_recv() {
            deltas.push(delta);
        }
        deltas
    }

    #[test]
    fn stream_capture_truncates_oversized_chunk_with_notice() {
        let mut capture = StreamCapture::new(ToolOutputStream::Stdout, 5);

        let emitted = capture.push_chunk("abcdef");

        assert_eq!(
            emitted,
            "abcde\n... [stdout truncated after 5 bytes; later output omitted]\n"
        );
        assert_eq!(capture.text, emitted);
        assert_eq!(capture.bytes_read, 6);
        assert!(capture.truncated);
    }

    #[test]
    fn stream_capture_emits_notice_when_next_chunk_crosses_limit_boundary() {
        let mut capture = StreamCapture::new(ToolOutputStream::Stderr, 5);

        assert_eq!(capture.push_chunk("hello"), "hello");
        assert!(!capture.truncated);
        let emitted = capture.push_chunk("!");

        assert_eq!(
            emitted,
            "\n... [stderr truncated after 5 bytes; later output omitted]\n"
        );
        assert_eq!(
            capture.text,
            "hello\n... [stderr truncated after 5 bytes; later output omitted]\n"
        );
        assert_eq!(capture.bytes_read, 6);
        assert!(capture.truncated);
    }

    #[tokio::test]
    async fn spawn_stream_reader_streams_long_lines_without_newlines_progressively() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let reader = ChunkedReader::new(vec![vec![b'a'; 5000]]);
        let ctx = test_tool_context_for(std::env::temp_dir()).with_tool_output_sender(tx);

        let handle = spawn_stream_reader(
            reader,
            ToolOutputStream::Stdout,
            ctx,
            "call-long".to_string(),
            "shell".to_string(),
            6000,
        );
        let capture = handle
            .join()
            .expect("reader thread should join")
            .expect("reader should succeed");
        let deltas = collect_output_deltas(&mut rx);

        assert_eq!(capture.text.len(), 5000);
        assert_eq!(capture.bytes_read, 5000);
        assert_eq!(
            deltas.len(),
            2,
            "4096 boundary should force an intermediate flush"
        );
        assert_eq!(deltas[0].delta.len(), 4096);
        assert_eq!(deltas[1].delta.len(), 904);
        assert!(deltas
            .iter()
            .all(|delta| delta.stream == ToolOutputStream::Stdout));
    }

    #[tokio::test]
    async fn spawn_stream_reader_preserves_utf8_chars_split_across_reads() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let reader = ChunkedReader::new(vec![
            vec![0xE4, 0xBD],
            vec![0xA0, 0xE5, 0xA5],
            vec![0xBD, b'\n'],
        ]);
        let ctx = test_tool_context_for(std::env::temp_dir()).with_tool_output_sender(tx);

        let handle = spawn_stream_reader(
            reader,
            ToolOutputStream::Stdout,
            ctx,
            "call-utf8".to_string(),
            "shell".to_string(),
            100,
        );
        let capture = handle
            .join()
            .expect("reader thread should join")
            .expect("reader should succeed");
        let deltas = collect_output_deltas(&mut rx);

        assert_eq!(capture.text, "你好\n");
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].delta, "你好\n");
    }

    #[tokio::test]
    async fn shell_tool_runs_non_interactive_command() {
        let tool = ShellTool;
        let args = if cfg!(windows) {
            json!({"command": "Write-Output 'ok'"})
        } else {
            json!({"command": "echo ok"})
        };

        let result = tool
            .execute(
                "tc1".to_string(),
                args,
                &test_tool_context_for(std::env::temp_dir()),
            )
            .await
            .expect("shell tool should execute");

        assert!(result.ok);
        assert!(result.output.contains("ok"));
    }

    #[tokio::test]
    async fn shell_tool_rejects_blank_command() {
        let tool = ShellTool;
        let err = tool
            .execute(
                "tc2".to_string(),
                json!({"command": "   "}),
                &test_tool_context_for(std::env::temp_dir()),
            )
            .await
            .expect_err("blank command should fail");

        assert!(matches!(err, AstrError::Validation(_)));
    }
}
