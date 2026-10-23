//! The dashboard's stock ticker: quotes from Finnhub (finnhub.io/docs/api),
//! fetched in the background every few minutes and cached in
//! `run/ticker.json`, so a restarted dashboard shows prices at once. Off
//! until a key is set (`[ticker]` in config.toml).

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{Paths, TickerConfig};

const API: &str = "https://finnhub.io/api/v1";
/// How often prices are fetched while they move, and while the market is
/// shut.
const EVERY: Duration = Duration::from_secs(120);
const CLOSED_EVERY: Duration = Duration::from_secs(15 * 60);
/// Between one symbol's request and the next: Finnhub's free plan allows
/// 60 a minute.
const SPACING: Duration = Duration::from_millis(250);
/// How often the config is looked at for a change.
const CONFIG_EVERY: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quote {
    pub symbol: String,
    pub price: f64,
    /// The day's change, in percent.
    pub change_pct: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Market {
    Open,
    /// Pre-market trading.
    Pre,
    /// After-hours trading.
    After,
    Closed,
    /// Its status couldn't be fetched.
    Unknown,
}

/// The prices the ticker shows, in the config's order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tape {
    pub indexes: Vec<Quote>,
    pub symbols: Vec<Quote>,
    pub market: Market,
    /// When it was last fetched.
    pub at: DateTime<Utc>,
}

#[derive(Debug, PartialEq)]
pub enum FetchError {
    /// Over the plan's limit: stop for this round.
    RateLimited,
    Other(String),
}

/// A `/quote` answer: None for a symbol Finnhub doesn't know (it answers
/// with zeros).
pub fn parse_quote(symbol: &str, json: &str) -> Option<Quote> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let price = v["c"].as_f64().filter(|c| *c > 0.0)?;
    let change_pct = v["dp"].as_f64().unwrap_or_else(|| {
        let pc = v["pc"].as_f64().unwrap_or(0.0);
        if pc > 0.0 {
            (price - pc) / pc * 100.0
        } else {
            0.0
        }
    });
    Some(Quote {
        symbol: symbol.to_string(),
        price,
        change_pct,
    })
}

/// A `/stock/market-status` answer.
pub fn parse_market(json: &str) -> Market {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Market::Unknown;
    };
    if v["isOpen"].as_bool() == Some(true) {
        return Market::Open;
    }
    match v["session"].as_str() {
        Some("pre-market") => Market::Pre,
        Some("post-market") => Market::After,
        _ if v["isOpen"].is_boolean() => Market::Closed,
        _ => Market::Unknown,
    }
}

/// `s` safe in a query string.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// One round: every configured symbol through `get` (an API path, e.g.
/// `/quote?symbol=AAPL`). A symbol that fails keeps its price from `prev`;
/// None when there's nothing to show at all.
pub fn fetch_tape(
    cfg: &TickerConfig,
    prev: Option<&Tape>,
    now: DateTime<Utc>,
    get: &mut dyn FnMut(&str) -> Result<String, FetchError>,
) -> Option<Tape> {
    let mut limited = false;
    let market = match get("/stock/market-status?exchange=US") {
        Ok(json) => parse_market(&json),
        Err(e) => {
            limited = e == FetchError::RateLimited;
            prev.map_or(Market::Unknown, |p| p.market)
        }
    };
    let mut fetch = |symbols: &[String], old: Option<&Vec<Quote>>| -> Vec<Quote> {
        symbols
            .iter()
            .filter_map(|sym| {
                let fresh = if limited {
                    None
                } else {
                    match get(&format!("/quote?symbol={}", encode(sym))) {
                        Ok(json) => parse_quote(sym, &json),
                        Err(e) => {
                            limited = e == FetchError::RateLimited;
                            None
                        }
                    }
                };
                fresh.or_else(|| old?.iter().find(|q| q.symbol == *sym).cloned())
            })
            .collect()
    };
    let indexes = fetch(&cfg.indexes, prev.map(|p| &p.indexes));
    let symbols = fetch(&cfg.symbols, prev.map(|p| &p.symbols));
    if indexes.is_empty() && symbols.is_empty() {
        return None;
    }
    Some(Tape {
        indexes,
        symbols,
        market,
        at: now,
    })
}

/// curl's arguments and its stdin for `url`: the key goes in a header
/// through stdin, out of the process list. The last line of curl's output
/// is the HTTP status.
pub fn curl_request(key: &str, url: &str) -> (Vec<String>, String) {
    let args = [
        "-sS",
        "--max-time",
        "10",
        "-K",
        "-",
        "-w",
        "\n%{http_code}",
        url,
    ]
    .map(String::from)
    .to_vec();
    let key: String = key.chars().filter(char::is_ascii_alphanumeric).collect();
    (args, format!("header = \"X-Finnhub-Token: {key}\"\n"))
}

/// GET `path` from Finnhub with curl.
fn get(key: &str, path: &str) -> Result<String, FetchError> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let (args, stdin) = curl_request(key, &format!("{API}{path}"));
    let mut child = Command::new("curl")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| FetchError::Other(format!("running curl: {e}")))?;
    if let Some(mut s) = child.stdin.take() {
        let _ = s.write_all(stdin.as_bytes());
    }
    let out = child
        .wait_with_output()
        .map_err(|e| FetchError::Other(e.to_string()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    match code.trim() {
        "200" => Ok(body.to_string()),
        "429" => Err(FetchError::RateLimited),
        c => Err(FetchError::Other(failure(c, body, &out.stderr))),
    }
}

/// Why a request failed: Finnhub's own `error`, else curl's, else the
/// HTTP status.
fn failure(code: &str, body: &str, stderr: &[u8]) -> String {
    let said = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_string));
    let curl = String::from_utf8_lossy(stderr).trim().to_string();
    said.or_else(|| (!curl.is_empty()).then_some(curl))
        .unwrap_or_else(|| format!("HTTP {code}"))
}

pub fn load(path: &Path) -> Option<Tape> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn save(path: &Path, tape: &Tape) {
    if let Ok(json) = serde_json::to_vec(tape) {
        let _ = crate::store::write_atomic(path, &json);
    }
}

/// What the ticker thread sends the dashboard.
#[derive(Debug, PartialEq)]
pub enum Update {
    /// No key or no symbols: hide the line.
    Off,
    Prices(Tape),
    /// Nothing to show, and why.
    Failed(String),
}

/// The key to fetch with, when the ticker is on.
fn wanted(cfg: &TickerConfig) -> Option<String> {
    cfg.key()
        .filter(|_| !(cfg.indexes.is_empty() && cfg.symbols.is_empty()))
}

/// Background thread fetching the tape. It reads `[ticker]` from
/// config.toml every round, and while waiting, so a new key or symbol list
/// shows without a restart.
pub struct Ticker {
    pub results: mpsc::Receiver<Update>,
}

impl Ticker {
    pub fn start(paths: Paths) -> Self {
        let (tx, results) = mpsc::channel();
        std::thread::spawn(move || {
            let cache = paths.run.join("ticker.json");
            let config = || {
                crate::config::load_config(&paths)
                    .map(|c| c.ticker)
                    .unwrap_or_default()
            };
            let mut prev: Option<Tape> = None;
            let mut on = false;
            loop {
                let cfg = config();
                let Some(key) = wanted(&cfg) else {
                    if std::mem::take(&mut on) && tx.send(Update::Off).is_err() {
                        return;
                    }
                    std::thread::sleep(CONFIG_EVERY);
                    continue;
                };
                if !std::mem::replace(&mut on, true) {
                    prev = load(&cache);
                    if let Some(t) = &prev
                        && tx.send(Update::Prices(t.clone())).is_err()
                    {
                        return;
                    }
                }
                let mut first = true;
                let mut why = None;
                let tape = fetch_tape(&cfg, prev.as_ref(), Utc::now(), &mut |path| {
                    if !std::mem::take(&mut first) {
                        std::thread::sleep(SPACING);
                    }
                    let got = get(&key, path);
                    if let Err(FetchError::Other(e)) = &got {
                        why = Some(e.clone());
                    }
                    got
                });
                let sent = match tape {
                    Some(t) => {
                        save(&cache, &t);
                        prev = Some(t.clone());
                        tx.send(Update::Prices(t))
                    }
                    None => tx.send(Update::Failed(why.unwrap_or_else(|| "no prices".into()))),
                };
                if sent.is_err() {
                    return; // the dashboard has gone
                }
                let closed = prev.as_ref().is_some_and(|t| t.market == Market::Closed);
                let mut wait = if closed { CLOSED_EVERY } else { EVERY };
                // Sooner when the config changes.
                while !wait.is_zero() && config() == cfg {
                    let nap = wait.min(CONFIG_EVERY);
                    std::thread::sleep(nap);
                    wait -= nap;
                }
            }
        });
        Self { results }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(indexes: &[&str], symbols: &[&str]) -> TickerConfig {
        TickerConfig {
            key: "k".into(),
            indexes: indexes.iter().map(|s| s.to_string()).collect(),
            symbols: symbols.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn q(symbol: &str, price: f64, change_pct: f64) -> Quote {
        Quote {
            symbol: symbol.into(),
            price,
            change_pct,
        }
    }

    #[test]
    fn quotes_parse_and_unknown_symbols_are_none() {
        let json = r#"{"c":261.74,"d":2.29,"dp":0.8826,"h":263.31,"l":260.68,"o":261.07,"pc":259.45,"t":1582641000}"#;
        assert_eq!(parse_quote("AAPL", json), Some(q("AAPL", 261.74, 0.8826)));
        // No dp: worked out from the previous close.
        let got = parse_quote("X", r#"{"c":110,"pc":100}"#).unwrap();
        assert!((got.change_pct - 10.0).abs() < 1e-9);
        let unknown = r#"{"c":0,"d":null,"dp":null,"h":0,"l":0,"o":0,"pc":0,"t":0}"#;
        assert_eq!(parse_quote("NOPE", unknown), None);
        assert_eq!(parse_quote("X", "error"), None);
    }

    #[test]
    fn market_status_parses() {
        let m = |s| parse_market(s);
        assert_eq!(
            m(r#"{"exchange":"US","isOpen":true,"session":"regular"}"#),
            Market::Open
        );
        assert_eq!(m(r#"{"isOpen":false,"session":"pre-market"}"#), Market::Pre);
        assert_eq!(
            m(r#"{"isOpen":false,"session":"post-market"}"#),
            Market::After
        );
        assert_eq!(m(r#"{"isOpen":false,"session":null}"#), Market::Closed);
        assert_eq!(m("nope"), Market::Unknown);
    }

    #[test]
    fn a_round_fetches_every_symbol_in_order() {
        let mut asked = Vec::new();
        let tape = fetch_tape(
            &cfg(&["SPY"], &["AAPL", "BRK.B", "NOPE"]),
            None,
            Utc::now(),
            &mut |path| {
                asked.push(path.to_string());
                Ok(match path {
                    p if p.starts_with("/stock/market-status") => r#"{"isOpen":true}"#.into(),
                    "/quote?symbol=SPY" => r#"{"c":500,"dp":0.4}"#.into(),
                    "/quote?symbol=AAPL" => r#"{"c":231.4,"dp":1.2}"#.into(),
                    "/quote?symbol=BRK.B" => r#"{"c":480,"dp":-0.3}"#.into(),
                    _ => r#"{"c":0,"dp":null}"#.into(),
                })
            },
        )
        .unwrap();
        assert_eq!(tape.market, Market::Open);
        assert_eq!(tape.indexes, [q("SPY", 500.0, 0.4)]);
        assert_eq!(
            tape.symbols,
            [q("AAPL", 231.4, 1.2), q("BRK.B", 480.0, -0.3)]
        );
        assert_eq!(asked.len(), 5);
        assert_eq!(encode("^GSPC"), "%5EGSPC");
    }

    #[test]
    fn failures_keep_the_last_prices_and_a_rate_limit_stops_the_round() {
        let prev = Tape {
            indexes: vec![],
            symbols: vec![q("AAPL", 230.0, 0.5), q("MSFT", 400.0, -1.0)],
            market: Market::Open,
            at: Utc::now(),
        };
        let mut asked = 0;
        let tape = fetch_tape(
            &cfg(&[], &["AAPL", "MSFT", "NVDA"]),
            Some(&prev),
            Utc::now(),
            &mut |path| {
                asked += 1;
                match path {
                    "/quote?symbol=AAPL" => Ok(r#"{"c":231.4,"dp":1.2}"#.into()),
                    "/quote?symbol=MSFT" => Err(FetchError::RateLimited),
                    p if p.starts_with("/stock") => Err(FetchError::Other("down".into())),
                    _ => panic!("asked after the rate limit: {path}"),
                }
            },
        )
        .unwrap();
        assert_eq!(asked, 3);
        assert_eq!(tape.market, Market::Open);
        assert_eq!(
            tape.symbols,
            [q("AAPL", 231.4, 1.2), q("MSFT", 400.0, -1.0)]
        );
        let nothing = fetch_tape(&cfg(&[], &["AAPL"]), None, Utc::now(), &mut |_| {
            Err(FetchError::Other("offline".into()))
        });
        assert_eq!(nothing, None);
    }

    #[test]
    fn the_key_stays_out_of_curls_arguments() {
        let (args, stdin) = curl_request("abc123", "https://finnhub.io/api/v1/quote?symbol=AAPL");
        assert!(args.iter().all(|a| !a.contains("abc123")), "{args:?}");
        assert_eq!(stdin, "header = \"X-Finnhub-Token: abc123\"\n");
        assert_eq!(
            args.last().unwrap(),
            "https://finnhub.io/api/v1/quote?symbol=AAPL"
        );
        // Nothing in a key can break out of curl's config line.
        let (_, stdin) = curl_request("ab\"c\nurl = x", "u");
        assert_eq!(stdin, "header = \"X-Finnhub-Token: abcurlx\"\n");
    }

    #[test]
    fn failures_say_why() {
        assert_eq!(
            failure("401", r#"{"error":"Invalid API key."}"#, b""),
            "Invalid API key."
        );
        assert_eq!(
            failure("000", "", b"curl: (6) Could not resolve host"),
            "curl: (6) Could not resolve host"
        );
        assert_eq!(failure("503", "<html>", b""), "HTTP 503");
    }

    #[test]
    fn the_cache_round_trips() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("ticker.json");
        assert_eq!(load(&path), None);
        let tape = Tape {
            indexes: vec![q("SPY", 500.0, 0.4)],
            symbols: vec![q("AAPL", 231.4, 1.2)],
            market: Market::After,
            at: Utc::now(),
        };
        save(&path, &tape);
        assert_eq!(load(&path), Some(tape));
    }

    #[test]
    fn on_with_a_key_and_something_to_show() {
        assert_eq!(wanted(&cfg(&["SPY"], &[])).as_deref(), Some("k"));
        assert_eq!(wanted(&cfg(&[], &["AAPL"])).as_deref(), Some("k"));
        assert_eq!(wanted(&cfg(&[], &[])), None);
        // Without one in the config it's the environment's.
        let mut c = cfg(&["SPY"], &[]);
        c.key.clear();
        assert_eq!(
            wanted(&c),
            std::env::var("FINNHUB_API_KEY")
                .ok()
                .filter(|k| !k.trim().is_empty())
                .map(|k| k.trim().to_string())
        );
    }
}
