#![cfg(feature = "native")]
// Native documentation drift fixture, outside the WASM-clean core.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use std::path::Path;

#[test]
fn skill_and_book_share_the_embedded_primer() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let primer = std::fs::read_to_string(root.join("src/native/primer.md")).unwrap();
    let mut previous = 0;
    for section in [
        "What Seeds is",
        "Agent workflow",
        "Core commands",
        "MCP tools",
        "Next steps",
    ] {
        let pos = primer.find(&format!("## {section}\n")).unwrap();
        assert!(pos > previous);
        previous = pos;
    }
    let path = root.join("skills/seeds/SKILL.md");
    let skill = std::fs::read_to_string(&path).unwrap();
    let marker = "<!-- BEGIN sd prime -->\n";
    let start = skill.find(marker).unwrap() + marker.len();
    let end = skill.find("<!-- END sd prime -->").unwrap();
    let expected = primer
        .lines()
        .map(|l| {
            if l.starts_with('#') {
                format!("##{l}\n")
            } else {
                format!("{l}\n")
            }
        })
        .collect::<String>();
    if std::env::var_os("SEEDS_BLESS_PRIMER").is_some() {
        std::fs::write(
            path,
            format!("{}{expected}{}", &skill[..start], &skill[end..]),
        )
        .unwrap();
    } else {
        assert_eq!(
            &skill[start..end],
            expected,
            "Run SEEDS_BLESS_PRIMER=1 just prime test and commit the generated copy"
        );
    }
    let page = std::fs::read_to_string(root.join("docs/book/src/agent-primer.md")).unwrap();
    assert!(page.contains("{{#include ../../../src/native/primer.md}}"));
}
