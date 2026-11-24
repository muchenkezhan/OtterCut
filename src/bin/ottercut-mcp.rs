use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use serde_json::{Value, json};

#[derive(Debug, Parser)]
#[command(name = "ottercut-mcp", about = "OtterCut 的本地 MCP stdio 服务")]
struct Arguments {
    /// 要提供给 MCP 工具读取的 .takeproj 工程文件
    #[arg(long)]
    project: PathBuf,
}

fn main() -> Result<()> {
    let arguments = Arguments::parse();
    if !arguments.project.is_file() {
        bail!("工程文件不存在：{}", arguments.project.display());
    }
    run_stdio(&arguments.project)
}

fn run_stdio(project: &std::path::Path) -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line.context("读取 MCP 请求失败")?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                write_response(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "id": Value::Null,
                        "error": {"code": -32700, "message": error.to_string()}
                    }),
                )?;
                continue;
            }
        };
        if request.get("id").is_none() {
            continue;
        }
        let response = handle_request(project, &request);
        write_response(&mut stdout, response)?;
    }
    Ok(())
}

fn handle_request(project: &std::path::Path, request: &Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": request
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2025-11-25"),
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "ottercut", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "读取已指定的 OtterCut 工程时间线"
        })),
        "tools/list" => Ok(json!({"tools": [get_timeline_tool()]})),
        "tools/call" => call_tool(project, request),
        "notifications/initialized" => Ok(Value::Null),
        _ => Err((-32601, format!("未知 MCP 方法：{method}"))),
    };
    match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": code, "message": message}
        }),
    }
}

fn get_timeline_tool() -> Value {
    json!({
        "name": "get_timeline",
        "description": "读取已连接 OtterCut 工程的时间线、轨道和片段信息",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
    })
}

fn call_tool(project: &std::path::Path, request: &Value) -> Result<Value, (i64, String)> {
    let name = request
        .pointer("/params/name")
        .and_then(Value::as_str)
        .ok_or((-32602, "tools/call 缺少工具名称".to_owned()))?;
    if name != "get_timeline" {
        return Err((-32602, format!("未知工具：{name}")));
    }
    let snapshot = takeseam::mcp::read_timeline(project)
        .map_err(|error| (-32000, format!("读取时间线失败：{error:#}")))?;
    let text = serde_json::to_string_pretty(&snapshot)
        .map_err(|error| (-32000, format!("序列化时间线失败：{error}")))?;
    Ok(json!({"content": [{"type": "text", "text": text}], "isError": false}))
}

fn write_response(stdout: &mut impl Write, response: Value) -> Result<()> {
    serde_json::to_writer(&mut *stdout, &response)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}
