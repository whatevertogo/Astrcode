use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crate::tools::fs_common::{check_cancel, resolve_path};
use astrcode_core::{
    AstrError, Result, SideEffectLevel, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult,
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
        let cwd = match args.cwd {
            Some(cwd) => resolve_path(ctx, &cwd)?,
            None => ctx.working_dir().clone(),
        };

        let mut child = Command::new(&spec.program)
            .args(&spec.args)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| AstrError::io("failed to spawn shell command", e))?;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| AstrError::Internal("failed to capture stdout".to_string()))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| AstrError::Internal("failed to capture stderr".to_string()))?;

        let stdout_task = thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes)?;
            std::result::Result::<Vec<u8>, std::io::Error>::Ok(bytes)
        });
        let stderr_task = thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes)?;
            std::result::Result::<Vec<u8>, std::io::Error>::Ok(bytes)
        });
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

        let stdout: Vec<u8> = stdout_task
            .join()
            .map_err(|_| AstrError::Internal("stdout reader thread panicked".to_string()))?
            .map_err(|e| AstrError::io("failed to read stdout", e))?;
        let stderr: Vec<u8> = stderr_task
            .join()
            .map_err(|_| AstrError::Internal("stderr reader thread panicked".to_string()))?
            .map_err(|e| AstrError::io("failed to read stderr", e))?;

        let stdout_text = String::from_utf8_lossy(&stdout).to_string();
        let stderr_text = String::from_utf8_lossy(&stderr).to_string();
        let exit_code = status.code().unwrap_or(-1);
        let ok = status.success();

        // Build output JSON and check size
        let output_json = json!({
            "stdout": stdout_text,
            "stderr": stderr_text,
            "exitCode": exit_code,
        });
        let output = output_json.to_string();

        // Truncate if output exceeds max size
        let (output, truncated) = if output.len() > ctx.max_output_size() {
            let truncation_msg = format!(
                "\n... [OUTPUT TRUNCATED: {} bytes total, showing first {} bytes]",
                output.len(),
                ctx.max_output_size()
            );
            let mut truncated_output =
                output[..ctx.max_output_size().saturating_sub(truncation_msg.len())].to_string();
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
            metadata: Some(json!({ "exitCode": exit_code, "truncated": truncated })),
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
        return CommandSpec {
            program,
            args: vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                command.to_string(),
            ],
        };
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_tool_context_for;

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
