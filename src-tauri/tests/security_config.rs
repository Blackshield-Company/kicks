//! Guards on the shipped Tauri security config.

fn csp() -> String {
    let raw = include_str!("../tauri.conf.json");
    let v: serde_json::Value = serde_json::from_str(raw).expect("valid tauri.conf.json");
    v["app"]["security"]["csp"]
        .as_str()
        .expect("csp is set")
        .to_string()
}

fn directive(csp: &str, name: &str) -> Option<String> {
    csp.split(';')
        .map(str::trim)
        .find(|d| d.starts_with(name))
        .map(str::to_string)
}

#[test]
fn csp_disallows_eval_and_inline_scripts() {
    let script = directive(&csp(), "script-src").expect("script-src present");
    assert!(!script.contains("unsafe-eval"), "{script}");
    assert!(!script.contains("unsafe-inline"), "{script}");
}

#[test]
fn csp_allows_tauri_ipc_only() {
    let connect = directive(&csp(), "connect-src").expect("connect-src present");
    assert!(
        connect.contains("ipc:") && connect.contains("http://ipc.localhost"),
        "{connect}"
    );
    assert!(
        !connect.contains("https:") && !connect.contains('*'),
        "{connect}"
    );
}

#[test]
fn csp_blocks_plugins_and_framing() {
    let c = csp();
    assert_eq!(
        directive(&c, "object-src").as_deref(),
        Some("object-src 'none'")
    );
    assert!(directive(&c, "frame-ancestors").is_some());
}
