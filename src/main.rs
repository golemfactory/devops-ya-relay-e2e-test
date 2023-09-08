use anyhow::{bail, Context, Result};
use lazy_static::lazy_static;
use regex::Regex;
use serde::Deserialize;
use std::{env, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::broadcast,
    time::timeout,
};

struct ChildWithMetadata {
    _child: Child,
    node_id: String,
}

#[derive(Deserialize)]
struct PingResponse {
    node_id: String,
    duration: i32,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let base_api_port = env::var("API_PORT")
        .context("could not get env API_PORT")?
        .parse::<i32>()
        .context("could not parse API_PORT")?;

    log::info!("Spawning children");
    let (child0, child1) = tokio::join!(
        spawn_http_client(base_api_port, 0),
        spawn_http_client(base_api_port, 1)
    );
    let (_child0, child1) = (
        child0.context("spawning child 0 failed")?,
        child1.context("spawning child 1 failed")?,
    );

    // "unreliable" is well... unreliable
    for transport in ["reliable", "transfer"] {
        log::info!("sending {transport} ping");
        let ping_result = timeout(
            Duration::from_secs(1),
            ping(base_api_port, transport, &child1.node_id),
        )
        .await
        .context("ping timeout")?
        .context("ping failed")?;

        if ping_result.node_id != child1.node_id {
            bail!(
                "Unexpected ping response. Got node id: {}. Expected {}",
                ping_result.node_id,
                child1.node_id,
            );
        }
        log::info!(
            "{transport} ping success, duration: {}",
            ping_result.duration
        );
    }

    Ok(())
}

async fn ping(api_port: i32, transport: &str, node_id: &str) -> Result<PingResponse> {
    Ok(reqwest::get(format!(
        "http://127.0.0.1:{api_port}/ping/{transport}/{}",
        node_id
    ))
    .await?
    .json::<PingResponse>()
    .await?)
}

async fn spawn_http_client(base_api_port: i32, child_id: i32) -> Result<ChildWithMetadata> {
    let mut child = Command::new("./http_client")
        .env("RUST_LOG", "info")
        .env("API_PORT", format!("{}", base_api_port + child_id))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("spawning http_client failed")?;

    let (tx, mut rx1) = broadcast::channel::<String>(16);
    let mut rx2 = tx.subscribe();

    tokio::spawn(async move {
        while let Ok(line) = rx1.recv().await {
            log::debug!("http_client {child_id}: {line}");
        }
    });

    let mut reader = BufReader::new(child.stderr.take().unwrap()).lines();
    tokio::spawn(async move {
        while let Ok(Some(line)) = reader.next_line().await {
            if tx.send(line).is_err() {
                return;
            }
        }
    });

    let node_id = timeout(
        Duration::from_secs(60),
        wait_until_child_ready(&mut rx2, child_id),
    )
    .await
    .context(format!("Child {child_id} setup timeout"))?;

    Ok(ChildWithMetadata {
        _child: child,
        node_id,
    })
}

// returns node_id
async fn wait_until_child_ready(rx: &mut broadcast::Receiver<String>, child_id: i32) -> String {
    lazy_static! {
        static ref ESTABLISHED_SESSION_RE: Regex =
            Regex::new(r"Established session [0-9a-f]{32} with NET relay server").unwrap();
        static ref NODE_ID_RE: Regex = Regex::new(r"CLIENT NODE ID: (0x[0-9a-f]{40})").unwrap();
    }
    static TOKIO_READY: &str = "Tokio runtime found";

    let mut established_session = false;
    let mut node_id: Option<String> = None;
    let mut tokio_ready = false;

    while let Ok(line) = rx.recv().await {
        if ESTABLISHED_SESSION_RE.is_match(&line) {
            log::info!("child {child_id} session established");
            established_session = true;
        }
        if let Some(caps) = NODE_ID_RE.captures(&line) {
            let tmp_node_id = caps.get(1).unwrap().as_str().to_owned();
            log::info!("child {child_id} node id: {tmp_node_id}");
            node_id = Some(tmp_node_id);
        }
        if line.contains(TOKIO_READY) {
            log::info!("child {child_id} ready");
            tokio_ready = true;
        }

        if established_session && node_id.is_some() && tokio_ready {
            return node_id.unwrap();
        }
    }

    unreachable!();
}
