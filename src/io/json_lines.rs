// Task 80: Strict JSON-lines protocol handling
use serde_json::Value;
use tokio::io::{AsyncWriteExt, BufReader, AsyncBufReadExt};
use tokio::net::UnixStream;

pub async fn handle_json_lines(stream: UnixStream) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        match serde_json::from_str::<Value>(&line) {
            Ok(json) => {
                // valid JSON
                writer.write_all(b"{\"status\":\"ok\"}\n").await?;
            }
            Err(_) => {
                writer.write_all(b"{\"error\":\"invalid json\"}\n").await?;
            }
        }
    }
    Ok(())
}
