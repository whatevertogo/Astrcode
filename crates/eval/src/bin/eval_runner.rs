use std::{path::PathBuf, process::ExitCode, time::Duration};

use astrcode_eval::runner::{EvalRunner, EvalRunnerConfig};

const HELP_TEXT: &str = r#"astrcode-eval-runner

Usage:
  cargo run -p astrcode-eval -- \
    --server-url <url> \
    --session-storage-root <path> \
    --task-set <path> \
    [--baseline <path>] \
    [--concurrency <n>] \
    [--keep-workspace] \
    [--output <path>]

Options:
  --server-url             Server HTTP 地址
  --session-storage-root   session projects 根目录
  --task-set               task-set.yaml 路径
  --baseline               基线报告 JSON 路径
  --concurrency            并发执行任务数，默认 1
  --keep-workspace         保留隔离工作区
  --output                 报告输出路径；未指定时打印到 stdout
  --help                   显示帮助

Environment:
  ASTRCODE_EVAL_TOKEN      可选，设置 x-astrcode-token 认证头
"#;

#[derive(Default)]
struct CliArgs {
    server_url: Option<String>,
    session_storage_root: Option<PathBuf>,
    task_set: Option<PathBuf>,
    baseline: Option<PathBuf>,
    concurrency: Option<usize>,
    keep_workspace: bool,
    output: Option<PathBuf>,
    help: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        },
    }
}

async fn run() -> Result<(), String> {
    let args = parse_args(std::env::args().skip(1))?;
    if args.help {
        print!("{HELP_TEXT}");
        return Ok(());
    }

    let config = EvalRunnerConfig {
        server_url: args
            .server_url
            .ok_or_else(|| "缺少必填参数 --server-url".to_string())?,
        session_storage_root: args
            .session_storage_root
            .ok_or_else(|| "缺少必填参数 --session-storage-root".to_string())?,
        task_set: args
            .task_set
            .ok_or_else(|| "缺少必填参数 --task-set".to_string())?,
        baseline: args.baseline,
        concurrency: args.concurrency.unwrap_or(1),
        keep_workspace: args.keep_workspace,
        output: args.output,
        timeout: Duration::from_secs(300),
        poll_interval: Duration::from_millis(500),
        ..EvalRunnerConfig::default()
    };

    let output_path = config.output.clone();
    let report = EvalRunner::run(config)
        .await
        .map_err(|error| error.to_string())?;
    if let Some(output) = output_path {
        eprintln!("report written to {}", output.display());
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| format!("序列化报告失败: {error}"))?
        );
    }
    Ok(())
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<CliArgs, String> {
    let mut parsed = CliArgs::default();
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => parsed.help = true,
            "--server-url" => parsed.server_url = Some(next_value(&mut args, "--server-url")?),
            "--session-storage-root" => {
                parsed.session_storage_root = Some(PathBuf::from(next_value(
                    &mut args,
                    "--session-storage-root",
                )?))
            },
            "--task-set" => {
                parsed.task_set = Some(PathBuf::from(next_value(&mut args, "--task-set")?))
            },
            "--baseline" => {
                parsed.baseline = Some(PathBuf::from(next_value(&mut args, "--baseline")?))
            },
            "--concurrency" => {
                let value = next_value(&mut args, "--concurrency")?;
                parsed.concurrency = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| format!("--concurrency 必须是正整数，收到 '{value}'"))?,
                );
            },
            "--keep-workspace" => parsed.keep_workspace = true,
            "--output" => parsed.output = Some(PathBuf::from(next_value(&mut args, "--output")?)),
            other => return Err(format!("未知参数: {other}\n\n{HELP_TEXT}")),
        }
    }

    Ok(parsed)
}

fn next_value(
    args: &mut std::iter::Peekable<impl Iterator<Item = String>>,
    flag: &str,
) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} 需要一个值"))
}
