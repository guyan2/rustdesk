//! 自定义设备源：从私有门户同步设备清单到「最近会话」。
//!
//! 单人使用场景下，OSS 版没有账号体系与官方地址簿。本模块在启动后
//! 定时拉取私有门户的设备列表，把尚未存在于本地的设备写入 PeerConfig，
//! 使客户端打开即可看到服务器已知的全部设备，无需手工输入 ID。

use hbb_common::{config::PeerConfig, log};
use std::collections::HashSet;
use std::time::Duration;

const PEER_API: &str = "https://kz51.dpdns.org/api/peers";
const PEER_API_TOKEN: &str = "xvFqOPwLz/OwP/tSRSLL2cAINzuGZ6NzDvHEU5Ha9lE=";
const SYNC_INTERVAL: Duration = Duration::from_secs(300);
const FIRST_DELAY: Duration = Duration::from_secs(8);
const ALIAS_KEY: &str = "alias";

/// 启动后台同步线程。由 `main_init` 调用，进程内只调用一次。
pub fn start_sync() {
    std::thread::spawn(|| {
        std::thread::sleep(FIRST_DELAY);
        loop {
            match sync_once() {
                Ok(0) => {}
                Ok(n) => {
                    log::info!("[custom-peers] {} new device(s) imported", n);
                    crate::flutter_ffi::main_load_recent_peers();
                }
                Err(err) => log::warn!("[custom-peers] sync failed: {}", err),
            }
            std::thread::sleep(SYNC_INTERVAL);
        }
    });
}

fn sync_once() -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()?;

    let resp = client
        .get(PEER_API)
        .bearer_auth(PEER_API_TOKEN)
        .header("Accept", "application/json")
        .send()?;

    if !resp.status().is_success() {
        return Err(format!("http status {}", resp.status()).into());
    }

    let payload: serde_json::Value = resp.json()?;
    let list = payload
        .get("peers")
        .and_then(|v| v.as_array())
        .ok_or("missing 'peers' field")?;

    // 只补写本地不存在的设备，避免覆盖用户已保存的连接口令与显示设置。
    let existing: HashSet<String> = PeerConfig::get_vec_id_modified_time_path(&None)
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();

    let mut imported = 0usize;
    for item in list {
        let id = match item.get("id").and_then(|v| v.as_str()) {
            Some(id) if !id.is_empty() => id,
            _ => continue,
        };
        let alias = item
            .get("alias")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let hostname = item
            .get("hostname")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let existed = existing.contains(id);
        // 读取既有配置后再写回，避免覆盖用户保存的连接口令与显示设置。
        let mut cfg = PeerConfig::load(id);
        let mut changed = false;

        if alias.is_empty() {
            // 门户清空备注时，本地别名也要清除，否则会一直保留旧名称
            if cfg.options.remove(ALIAS_KEY).is_some() {
                changed = true;
            }
        } else if cfg.options.get(ALIAS_KEY) != Some(&alias) {
            cfg.options.insert(ALIAS_KEY.to_owned(), alias);
            changed = true;
        }
        if cfg.info.hostname.is_empty() && !hostname.is_empty() {
            cfg.info.hostname = hostname;
            changed = true;
        }

        if changed || !existed {
            cfg.store(id);
            imported += 1;
        }
    }

    Ok(imported)
}
