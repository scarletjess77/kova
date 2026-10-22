//! Client for Mihomo's external controller (REST + streaming endpoints).

use std::collections::HashMap;
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Error;

#[derive(Clone)]
pub struct Mihomo {
    base: String,
    secret: String,
    http: reqwest::Client,
    stream_http: reqwest::Client,
}

fn seg(s: &str) -> String {
    percent_encode(s)
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyGroup {
    pub name: String,
    /// `Selector`, `URLTest`, `Fallback`, `LoadBalance`, ...
    #[serde(rename = "type")]
    pub kind: String,
    pub now: String,
    pub all: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub udp: bool,
    pub alive: bool,
    /// Last measured delay in ms, `None` if never tested or failed.
    pub delay: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProxySnapshot {
    pub groups: Vec<ProxyGroup>,
    pub proxies: HashMap<String, ProxyInfo>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Traffic {
    pub up: u64,
    pub down: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub host: String,
    pub dst: String,
    pub port: String,
    pub network: String,
    pub process: String,
    pub rule: String,
    pub chains: Vec<String>,
    pub upload: u64,
    pub download: u64,
    pub start: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectionsSnapshot {
    pub upload_total: u64,
    pub download_total: u64,
    pub connections: Vec<Connection>,
}

impl Mihomo {
    pub fn new(port: u16, secret: &str) -> Self {
        let http = crate::fetch::builder().no_proxy().timeout(Duration::from_secs(15)).build().expect("client");
        let stream_http = crate::fetch::builder().no_proxy().build().expect("client");
        Self { base: format!("http://127.0.0.1:{port}"), secret: secret.to_string(), http, stream_http }
    }

    fn req(&self, c: &reqwest::Client, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        c.request(method, format!("{}{path}", self.base)).bearer_auth(&self.secret)
    }

    async fn json(&self, path: &str) -> Result<Value, Error> {
        let r = self.req(&self.http, reqwest::Method::GET, path).send().await?;
        let status = r.status();
        let v: Value = r.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(Error::Core(v["message"].as_str().unwrap_or(status.as_str()).to_string()));
        }
        Ok(v)
    }

    pub async fn version(&self) -> Result<String, Error> {
        Ok(self.json("/version").await?["version"].as_str().unwrap_or_default().to_string())
    }

    /// Wait until the controller answers, or give up after `timeout`.
    pub async fn wait_ready(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            if self.version().await.is_ok() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
        false
    }

    pub async fn proxies(&self) -> Result<ProxySnapshot, Error> {
        let v = self.json("/proxies").await?;
        let map = v["proxies"].as_object().cloned().unwrap_or_default();
        let mut snap = ProxySnapshot::default();
        for (name, p) in &map {
            let delay = p["history"].as_array().and_then(|h| h.last()).and_then(|h| h["delay"].as_u64()).filter(|d| *d > 0).map(|d| d as u32);
            snap.proxies.insert(
                name.clone(),
                ProxyInfo {
                    name: name.clone(),
                    kind: p["type"].as_str().unwrap_or_default().to_string(),
                    udp: p["udp"].as_bool().unwrap_or(false),
                    alive: p["alive"].as_bool().unwrap_or(true),
                    delay,
                },
            );
        }
        // GLOBAL lists every group in config order; use it to order the groups.
        let order: Vec<String> = map.get("GLOBAL").and_then(|g| g["all"].as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        let to_group = |name: &str, p: &Value| -> Option<ProxyGroup> {
            let all = p["all"].as_array()?;
            Some(ProxyGroup {
                name: name.to_string(),
                kind: p["type"].as_str().unwrap_or_default().to_string(),
                now: p["now"].as_str().unwrap_or_default().to_string(),
                all: all.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
            })
        };
        for name in order.iter().chain(std::iter::once(&"GLOBAL".to_string())) {
            if let Some(g) = map.get(name).and_then(|p| to_group(name, p)) {
                snap.groups.push(g);
            }
        }
        Ok(snap)
    }

    pub async fn select(&self, group: &str, name: &str) -> Result<(), Error> {
        let r = self.req(&self.http, reqwest::Method::PUT, &format!("/proxies/{}", seg(group))).json(&serde_json::json!({ "name": name })).send().await?;
        if r.status().is_success() { Ok(()) } else { Err(Error::Core(format!("could not select {name} in {group}"))) }
    }

    /// Measure one proxy. `Ok(None)` means it timed out or failed.
    pub async fn delay(&self, name: &str, url: &str, timeout_ms: u32) -> Result<Option<u32>, Error> {
        let path = format!("/proxies/{}/delay?timeout={timeout_ms}&url={}", seg(name), percent_encode(url));
        match self.json(&path).await {
            Ok(v) => Ok(v["delay"].as_u64().filter(|d| *d > 0).map(|d| d as u32)),
            Err(Error::Core(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Test every member of a group concurrently (done by Mihomo).
    pub async fn group_delay(&self, group: &str, url: &str, timeout_ms: u32) -> Result<HashMap<String, u32>, Error> {
        let path = format!("/group/{}/delay?timeout={timeout_ms}&url={}", seg(group), percent_encode(url));
        let v = self.json(&path).await.unwrap_or(Value::Null);
        Ok(v.as_object().map(|m| m.iter().filter_map(|(k, d)| d.as_u64().map(|d| (k.clone(), d as u32))).collect()).unwrap_or_default())
    }

    /// Resolve `name` the way Mihomo resolves for traffic: its addresses (fake ones in
    /// Fake-IP mode).
    pub async fn dns_query(&self, name: &str) -> Result<Vec<String>, Error> {
        let v = self.json(&format!("/dns/query?name={name}&type=A")).await?;
        Ok(v["Answer"].as_array().map(|a| a.iter().filter_map(|x| x["data"].as_str().map(str::to_string)).collect()).unwrap_or_default())
    }

    pub async fn connections(&self) -> Result<ConnectionsSnapshot, Error> {
        let v = self.json("/connections").await?;
        let conns = v["connections"].as_array().cloned().unwrap_or_default();
        Ok(ConnectionsSnapshot {
            upload_total: v["uploadTotal"].as_u64().unwrap_or(0),
            download_total: v["downloadTotal"].as_u64().unwrap_or(0),
            connections: conns
                .iter()
                .map(|c| {
                    let m = &c["metadata"];
                    let st = |k: &str| m[k].as_str().unwrap_or_default().to_string();
                    let host = [st("host"), st("sniffHost")].into_iter().find(|h| !h.is_empty()).unwrap_or_else(|| st("destinationIP"));
                    let rule = match (c["rule"].as_str(), c["rulePayload"].as_str()) {
                        (Some(r), Some(p)) if !p.is_empty() => format!("{r} {p}"),
                        (Some(r), _) => r.to_string(),
                        _ => String::new(),
                    };
                    Connection {
                        id: c["id"].as_str().unwrap_or_default().to_string(),
                        host,
                        dst: st("destinationIP"),
                        port: st("destinationPort"),
                        network: st("network").to_uppercase(),
                        process: st("process"),
                        rule,
                        // Mihomo lists the chain from the exit node back to the group.
                        chains: c["chains"].as_array().map(|a| a.iter().rev().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
                        upload: c["upload"].as_u64().unwrap_or(0),
                        download: c["download"].as_u64().unwrap_or(0),
                        start: c["start"].as_str().unwrap_or_default().to_string(),
                    }
                })
                .collect(),
        })
    }

    pub async fn close_connection(&self, id: &str) -> Result<(), Error> {
        self.req(&self.http, reqwest::Method::DELETE, &format!("/connections/{}", seg(id))).send().await?;
        Ok(())
    }

    pub async fn close_all(&self) -> Result<(), Error> {
        self.req(&self.http, reqwest::Method::DELETE, "/connections").send().await?;
        Ok(())
    }

    /// Make Mihomo fetch a rule provider again.
    pub async fn update_rule_provider(&self, name: &str) -> Result<(), Error> {
        self.req(&self.http, reqwest::Method::PUT, &format!("/providers/rules/{}", seg(name))).send().await?.error_for_status()?;
        Ok(())
    }

    pub async fn set_mode(&self, mode: kova_core::options::Mode) -> Result<(), Error> {
        self.req(&self.http, reqwest::Method::PATCH, "/configs").json(&serde_json::json!({ "mode": mode })).send().await?.error_for_status()?;
        Ok(())
    }

    /// Stream newline-delimited JSON from `path`, calling `f` for each object, until the
    /// connection drops or `f` returns false.
    pub async fn stream(&self, path: &str, mut f: impl FnMut(Value) -> bool) -> Result<(), Error> {
        let r = self.req(&self.stream_http, reqwest::Method::GET, path).send().await?.error_for_status()?;
        let mut body = r.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = body.next().await {
            buf.extend_from_slice(&chunk?);
            while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                if let Ok(v) = serde_json::from_slice::<Value>(&line) {
                    if !f(v) {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }
}
