use std::{
    process::{Command, Output, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    time::{timeout_at, Instant},
};

pub(super) const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

async fn read_process_output(mut pipe: impl AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = pipe.read(&mut buffer).await?;
        if count == 0 {
            return Ok(output);
        }
        // PicGo 日志不应无限驻留；保留最后 1 MiB（URL 通常在末尾），继续排空管道。
        let excess = (output.len() + count).saturating_sub(MAX_RESPONSE_BYTES);
        if excess > 0 {
            output.drain(..excess);
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

pub(super) async fn run_picgo_command(
    mut command: Command,
    timeout: Duration,
) -> Result<Output, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = tokio::process::Command::from(command)
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("调用 PicGo-Core 失败：{error}"))?;
    let mut stdout = tokio::spawn(read_process_output(
        child.stdout.take().ok_or("无法读取 PicGo-Core 输出")?,
    ));
    let mut stderr = tokio::spawn(read_process_output(
        child.stderr.take().ok_or("无法读取 PicGo-Core 错误输出")?,
    ));
    let deadline = Instant::now() + timeout;
    let mut stdout_finished = false;
    let mut stderr_finished = false;
    let result = timeout_at(deadline, async {
        let status = child.wait().await.map_err(|error| error.to_string())?;
        let stdout_result = (&mut stdout).await;
        stdout_finished = true;
        let stdout = stdout_result
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        let stderr_result = (&mut stderr).await;
        stderr_finished = true;
        let stderr = stderr_result
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(Output {
            status,
            stdout,
            stderr,
        })
    })
    .await;
    match result {
        Ok(Ok(output)) => Ok(output),
        failure => {
            // 只终止本次调用持有的子进程；不按名称终止用户的 PicGo 服务。
            let _ = child.kill().await;
            let _ = child.wait().await;
            // 另一条管道超时时，已取出的 JoinHandle 结果不能再次 await。
            if !stdout_finished {
                stdout.abort();
                let _ = stdout.await;
            }
            if !stderr_finished {
                stderr.abort();
                let _ = stderr.await;
            }
            match failure {
                Err(_) => Err(format!("PicGo-Core 上传超时（{} 秒）", timeout.as_secs())),
                Ok(Err(error)) => Err(format!("PicGo-Core 上传失败：{error}")),
                _ => unreachable!(),
            }
        }
    }
}

pub(super) async fn post_json(url: &str, body: &str, timeout: Duration) -> Result<String, String> {
    let client = reqwest::Client::builder()
        // 原 TCP 实现直接连接用户配置的 PicGo 地址，不让系统代理改变上传目的地。
        .no_proxy()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?;
    let mut response = client
        .post(url)
        .header("content-type", "application/json")
        .body(body.to_owned())
        .send()
        .await
        .map_err(|error| format!("请求 PicGo Server 失败：{error}"))?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(format!("PicGo Server 返回异常状态：{}", response.status()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("读取 PicGo Server 响应失败：{error}"))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err("PicGo Server 响应过大".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|error| format!("PicGo Server 响应不是 UTF-8：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    #[test]
    fn server_response_supports_chunked_encoding_and_has_a_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            stream.read(&mut buffer).unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\n{}\r\n0\r\n\r\n").unwrap();
        });
        assert_eq!(
            tauri::async_runtime::block_on(post_json(
                &format!("http://{address}"),
                "{}",
                Duration::from_secs(2)
            ))
            .unwrap(),
            "{}"
        );
        server.join().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(150));
        });
        assert!(tauri::async_runtime::block_on(post_json(
            &format!("http://{address}"),
            "{}",
            Duration::from_millis(40)
        ))
        .is_err());
        server.join().unwrap();
    }

    #[test]
    fn core_process_is_reaped_on_timeout() {
        #[cfg(windows)]
        let command = {
            let mut command = Command::new("pwsh");
            command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 10"]);
            command
        };
        #[cfg(not(windows))]
        let command = {
            let mut command = Command::new("sleep");
            command.arg("10");
            command
        };
        let started = std::time::Instant::now();
        assert!(tauri::async_runtime::block_on(run_picgo_command(
            command,
            Duration::from_millis(60)
        ))
        .unwrap_err()
        .contains("超时"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
