use std::path::PathBuf;

use manato_formats::{Character, EffectsBank, SoundBank, Stage};

fn mugen_root() -> Option<PathBuf> {
    let root = std::env::var_os("MANATO_MUGEN_ROOT")
        .or_else(|| std::env::var_os("MUGEN_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    root.exists().then_some(root)
}

#[test]
fn loads_stock_kfm_and_stage() {
    let Some(root) = mugen_root() else {
        eprintln!("skipping: M.U.G.E.N content root not available");
        return;
    };
    let kfm_def = root.join("mugen-1.1b1/chars/kfm/kfm.def");
    if !kfm_def.exists() {
        eprintln!("skipping: stock M.U.G.E.N content not available");
        return;
    }
    let data = root.join("mugen-1.1b1/data");
    let data_dirs = vec![data.clone(), root.join("mugen-1.1b1")];

    let kfm = Character::load(&kfm_def, &data_dirs).expect("KFM should load");
    assert!(kfm.states.contains_key(&0), "common state 0 missing");
    assert!(kfm.states.contains_key(&-1), "cmd state -1 missing");
    assert!(kfm.commands.iter().any(|c| c.name == "QCF_x"));
    assert!(kfm.animations.contains_key(&200), "animation 200 missing");
    assert!(!kfm.sprites.sprites.is_empty(), "no sprites decoded");
    assert!(!kfm.sounds.sounds.is_empty(), "no sounds decoded");

    let s = kfm.sprites.get(0, 0).expect("sprite 0,0 missing");
    assert!(s.width > 0 && s.height > 0);
    assert_eq!(s.rgba.len(), (s.width * s.height * 4) as usize);
    // Stock KFM's first sprite is fully opaque; transparency is per-pixel via
    // the palette index, not a global "index 0 is transparent" rule.
    assert!(kfm
        .sprites
        .sprites
        .iter()
        .any(|s| s.rgba.chunks_exact(4).any(|px| px[3] == 0)));

    let stage = Stage::load(&root.join("mugen-1.1b1/stages/kfm.def"), &data_dirs)
        .expect("KFM stage should load");
    assert!(!stage.layers.is_empty());

    let fx = EffectsBank::load(&[data]);
    assert!(!fx.sprites.sprites.is_empty());
    assert!(!fx.animations.is_empty());
}

#[test]
fn loads_original_and_community_content() {
    let Some(root) = mugen_root() else {
        eprintln!("skipping: M.U.G.E.N content root not available");
        return;
    };
    let data_dirs = vec![
        root.join("mugen-1.1b1/data"),
        root.join("mugen-1.1b1"),
        root.join("content/chars/ryu"),
    ];
    for (def, name) in [
        ("content/chars/kaen/kaen.def", "Kaen"),
        ("content/chars/nival/nival.def", "Nival"),
        ("content/chars/ordan/ordan.def", "Ordan"),
        ("content/chars/ryu/Ryu.def", "Ryu"),
    ] {
        let path = root.join(def);
        if !path.exists() {
            eprintln!("skipping missing {def}");
            continue;
        }
        let c = Character::load(&path, &data_dirs)
            .unwrap_or_else(|e| panic!("{name} should load: {e:#}"));
        assert!(!c.states.is_empty(), "{name} has no states");
        assert!(!c.animations.is_empty(), "{name} has no animations");
        assert!(!c.sprites.sprites.is_empty(), "{name} has no sprites");
        if name != "Ryu" {
            assert!(!c.sounds.sounds.is_empty(), "{name} has no sounds");
            for key in [(0, 0), (0, 2), (3, 0), (3, 1), (5, 2), (6, 0)] {
                let sound = c
                    .sounds
                    .get(key.0, key.1)
                    .unwrap_or_else(|| panic!("{name} missing sound {key:?}"));
                assert!(
                    sound.duration_seconds() > 0.0,
                    "{name} sound {key:?} is empty"
                );
            }
        }
        println!(
            "{name}: {} states, {} anims, {} sprites, {} sounds, {} commands",
            c.states.len(),
            c.animations.len(),
            c.sprites.sprites.len(),
            c.sounds.sounds.len(),
            c.commands.len()
        );
    }

    let stage_path = root.join("content/stages/training/stage1.def");
    if stage_path.exists() {
        let stage = Stage::load(&stage_path, &data_dirs).expect("training stage should load");
        assert!(!stage.layers.is_empty());
        assert!(!stage.sprites.sprites.is_empty());
    }
}

#[test]
fn loads_bundled_mit_assets() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = root.join("content");

    let fx = EffectsBank::load(&[content.clone()]);
    for action in [0, 1, 2, 40] {
        assert!(
            fx.animations.contains_key(&action),
            "missing fightfx action {action}"
        );
    }
    assert!(!fx.sprites.sprites.is_empty(), "fightfx sprites missing");

    let fight = SoundBank::load(&content.join("fight.snd")).expect("fight.snd");
    for key in [(0, 0), (0, 2), (1, 0), (2, 0), (21, 0)] {
        assert!(
            fight.get(key.0, key.1).is_some(),
            "missing fight sound {key:?}"
        );
    }

    let stage = Stage::load(
        &content.join("stages/training/stage1.def"),
        &[content.clone()],
    )
    .expect("training stage");
    assert!(stage.zoffset > 0.0);
    assert!(!stage.layers.is_empty());
    assert!(!stage.sprites.sprites.is_empty());
    assert_eq!(stage.name, "Training Grid");
}

#[test]
fn loads_stock_stages() {
    let Some(root) = mugen_root() else {
        eprintln!("skipping: M.U.G.E.N content root not available");
        return;
    };
    let stock = root.join("mugen-1.1b1");
    if !stock.exists() {
        eprintln!("skipping: stock M.U.G.E.N content not available");
        return;
    }
    let data_dirs = vec![stock.join("data"), stock.clone()];
    let mut loaded = 0;
    for name in ["kfm.def", "stage0.def", "stage0-720.def", "stage1.def"] {
        let path = stock.join("stages").join(name);
        if !path.exists() {
            continue;
        }
        let stage = Stage::load(&path, &data_dirs)
            .unwrap_or_else(|error| panic!("{name} should load: {error:#}"));
        assert!(!stage.layers.is_empty(), "{name} has no background layers");
        assert!(
            !stage.sprites.sprites.is_empty(),
            "{name} has no decoded sprites"
        );
        loaded += 1;
    }
    if loaded == 0 {
        eprintln!("skipping: no stock stages present");
    }
}
