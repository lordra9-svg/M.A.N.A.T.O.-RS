use std::path::PathBuf;

use manato_formats::screenpack::{Roster, Screenpack};

fn root() -> PathBuf {
    std::env::var_os("MANATO_MUGEN_ROOT")
        .or_else(|| std::env::var_os("MUGEN_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn grid() -> Option<PathBuf> {
    let path = root().join("content/screenpacks/manato-grid/system.def");
    path.exists().then_some(path)
}

#[test]
fn loads_bundled_manato_grid() {
    let Some(path) = grid() else {
        eprintln!("skipping: bundled screenpack not present");
        return;
    };
    let pack = Screenpack::load(&path).expect("Manato Grid should load");
    assert_eq!(pack.name, "Manato Grid");
    assert_eq!(pack.localcoord, (320.0, 240.0));
    assert!(!pack.sprites.sprites.is_empty());
    assert!(pack.loaded_fonts.contains_key(&0), "font0 should load");
    let font = &pack.loaded_fonts[&0];
    for ch in ['K', 'a', '0', '%', '.'] {
        assert!(font.glyph(ch).is_some(), "font is missing {ch:?}");
    }

    // `[LifeBarP2]` only restates `pos`; everything else comes from the base
    // `[LifeBar]` section.
    assert_eq!(pack.fight.life[1].pos.x, 182.0);
    assert_eq!(pack.fight.life[1].range.x, 132.0);
    assert_eq!(pack.fight.life[1].front.sprite, Some((0, 1)));
    assert_eq!(pack.fight.power[1].pos.x, 192.0);
    assert_eq!(pack.fight.power[1].range.x, 122.0);
    assert_eq!(pack.fight.name_def[1].name.align, 2);
    assert_eq!(pack.fight.combo[1].start_x, -48.0);
    assert_eq!(pack.fight.win_icon[1].pos.x, 70.0);

    assert!(pack.title_bg.is_some(), "title background missing");
    assert!(pack.select_bg.is_some(), "select background missing");
    assert!(pack.select.rows >= 1);
    assert!(pack.select.columns >= 1);
}

#[test]
fn loads_bundled_roster() {
    let Some(path) = grid() else {
        eprintln!("skipping: bundled screenpack not present");
        return;
    };
    let roster_path = path.parent().unwrap().join("select.def");
    let roster = Roster::load(&roster_path).expect("select.def should load");
    let defs: Vec<&str> = roster.entries.iter().map(|e| e.def.as_str()).collect();
    assert!(defs.contains(&"content/chars/kaen/kaen.def"));
    assert!(defs.contains(&"content/chars/nival/nival.def"));
    assert!(defs.contains(&"content/chars/ordan/ordan.def"));
}
