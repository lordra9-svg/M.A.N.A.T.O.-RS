use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::text::{parse_pair, TextFile};

#[derive(Debug, Clone, Default)]
pub struct CharacterFiles {
    pub cmd: Option<PathBuf>,
    pub cns: Option<PathBuf>,
    pub states: Vec<PathBuf>,
    pub common_states: Vec<PathBuf>,
    pub sprite: Option<PathBuf>,
    pub anim: Option<PathBuf>,
    pub sound: Option<PathBuf>,
    pub ai: Option<PathBuf>,
    /// `movelist`, `movelist0`, … selection files (`ChangeMovelist`).
    pub movelists: BTreeMap<i32, PathBuf>,
    /// `pal1`..`palN` ACT palettes keyed by their 1-based index.
    pub palettes: BTreeMap<i32, PathBuf>,
}

/// One `[RemapPreset <name>]` mapping table.
pub type RemapPreset = BTreeMap<(i32, i32), (i32, i32)>;

/// Parsed character `.def` file.
#[derive(Debug, Clone)]
pub struct CharacterDef {
    pub path: PathBuf,
    pub dir: PathBuf,
    pub name: String,
    pub display_name: String,
    pub author: String,
    pub mugen_version: String,
    pub localcoord: (f32, f32),
    pub files: CharacterFiles,
    pub info: HashMap<String, String>,
    pub palette_keymap: HashMap<String, i32>,
    /// `[RemapPreset <name>]` tables: source `(group, index)` → dest.
    pub remap_presets: BTreeMap<String, RemapPreset>,
    /// `[Shaders]` name → file path (names lowercased).
    pub shaders: BTreeMap<String, PathBuf>,
}

impl CharacterDef {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = TextFile::load(path)?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let info = text.find("info").map(section_map).unwrap_or_default();
        let files_section = text.find("files").map(section_map).unwrap_or_default();

        let name = info.get("name").cloned().unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Unknown".into())
        });
        let display_name = info
            .get("displayname")
            .cloned()
            .unwrap_or_else(|| name.clone());
        let author = info.get("author").cloned().unwrap_or_default();
        let mugen_version = info
            .get("mugenversion")
            .cloned()
            .unwrap_or_else(|| "1.0".into());
        let localcoord = info
            .get("localcoord")
            .map(|v| parse_pair(v))
            .filter(|(w, h)| *w > 0.0 && *h > 0.0)
            .unwrap_or((320.0, 240.0));

        let resolve = |key: &str| files_section.get(key).map(|v| resolve_path(&dir, v));
        // `st`, `st0`, `st1`, … `stN` are merged in natural numeric order, so
        // `st10` follows `st9` and numbers beyond 9 are supported.
        let mut state_keys: Vec<(i64, &String)> = files_section
            .iter()
            .filter_map(|(key, value)| {
                let suffix = key.strip_prefix("st")?;
                let order = if suffix.is_empty() {
                    -1
                } else if suffix.chars().all(|c| c.is_ascii_digit()) {
                    suffix.parse::<i64>().ok()?
                } else {
                    return None;
                };
                Some((order, value))
            })
            .collect();
        state_keys.sort_by_key(|(order, _)| *order);
        let mut states = Vec::new();
        for (_, value) in state_keys {
            for part in value.split(',') {
                let p = resolve_path(&dir, part);
                if !part.trim().is_empty() {
                    states.push(p);
                }
            }
        }
        let mut common_states = Vec::new();
        if let Some(v) = files_section.get("stcommon") {
            for part in v.split(',') {
                if part.trim().is_empty() {
                    continue;
                }
                common_states.push(resolve_path(&dir, part));
            }
        }

        let mut movelists = BTreeMap::new();
        for (key, value) in &files_section {
            let Some(suffix) = key.strip_prefix("movelist") else {
                continue;
            };
            let id = if suffix.is_empty() || suffix == "0" {
                0
            } else if suffix.chars().all(|c| c.is_ascii_digit()) {
                match suffix.parse::<i32>() {
                    Ok(id) => id,
                    Err(_) => continue,
                }
            } else {
                continue;
            };
            if !value.trim().is_empty() {
                movelists.insert(id, resolve_path(&dir, value));
            }
        }

        let mut palettes = BTreeMap::new();
        for (key, value) in &files_section {
            let Some(suffix) = key.strip_prefix("pal") else {
                continue;
            };
            if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            if let Ok(index) = suffix.parse::<i32>() {
                if index > 0 && !value.trim().is_empty() {
                    palettes.insert(index, resolve_path(&dir, value));
                }
            }
        }

        let files = CharacterFiles {
            cmd: resolve("cmd"),
            cns: resolve("cns"),
            states,
            common_states,
            sprite: resolve("sprite"),
            anim: resolve("anim"),
            sound: resolve("sound"),
            ai: resolve("ai"),
            movelists,
            palettes,
        };

        let mut remap_presets = BTreeMap::new();
        for section in &text.sections {
            let header = section.header_lower();
            let Some(name) = header
                .strip_prefix("remappreset ")
                .map(str::trim)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            let mut table = BTreeMap::new();
            for entry in &section.entries {
                let Some((src, dst)) = entry.key.split_once(',') else {
                    continue;
                };
                let (Ok(sg), Ok(si)) = (src.trim().parse::<i32>(), dst.trim().parse::<i32>())
                else {
                    continue;
                };
                let mut parts = entry.value.split(',');
                let (Some(dg), Some(di)) = (parts.next(), parts.next()) else {
                    continue;
                };
                let (Ok(dg), Ok(di)) = (dg.trim().parse::<i32>(), di.trim().parse::<i32>()) else {
                    continue;
                };
                table.insert((sg, si), (dg, di));
            }
            if !table.is_empty() {
                remap_presets.insert(name.to_string(), table);
            }
        }

        let mut shaders = BTreeMap::new();
        if let Some(section) = text.find("shaders") {
            for entry in &section.entries {
                let name = entry.key.trim().to_ascii_lowercase();
                let value = entry.value.trim();
                if !name.is_empty() && !value.is_empty() {
                    shaders.insert(name, resolve_path(&dir, value));
                }
            }
        }

        let palette_keymap = text
            .find_header("Palette Keymap")
            .map(|s| {
                s.entries
                    .iter()
                    .filter_map(|e| {
                        e.value
                            .trim()
                            .parse::<i32>()
                            .ok()
                            .map(|v| (e.key.trim().to_ascii_lowercase(), v))
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(Self {
            path: path.to_path_buf(),
            dir,
            name,
            display_name,
            author,
            mugen_version,
            localcoord,
            files,
            info,
            palette_keymap,
            remap_presets,
            shaders,
        })
    }
}

/// Parsed stage `.def` file, keeping the raw sections for the stage builder.
#[derive(Debug, Clone)]
pub struct StageDefFile {
    pub path: PathBuf,
    pub dir: PathBuf,
    pub name: String,
    pub display_name: String,
    pub author: String,
    pub mugen_version: String,
    pub localcoord: (f32, f32),
    pub info: HashMap<String, String>,
    pub camera: HashMap<String, String>,
    pub player_info: HashMap<String, String>,
    pub bgdef: HashMap<String, String>,
    pub text: TextFile,
}

impl StageDefFile {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = TextFile::load(path)?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let info = text.find("info").map(section_map).unwrap_or_default();
        let camera = text.find("camera").map(section_map).unwrap_or_default();
        let player_info = text.find("playerinfo").map(section_map).unwrap_or_default();
        let bgdef = text.find("bgdef").map(section_map).unwrap_or_default();
        let name = info.get("name").cloned().unwrap_or_else(|| "Stage".into());
        let display_name = info
            .get("displayname")
            .cloned()
            .unwrap_or_else(|| name.clone());
        let localcoord = info
            .get("localcoord")
            .map(|v| parse_pair(v))
            .filter(|(w, h)| *w > 0.0 && *h > 0.0)
            .unwrap_or((320.0, 240.0));
        Ok(Self {
            path: path.to_path_buf(),
            dir,
            name,
            display_name,
            author: info.get("author").cloned().unwrap_or_default(),
            mugen_version: info
                .get("mugenversion")
                .cloned()
                .unwrap_or_else(|| "1.0".into()),
            localcoord,
            info,
            camera,
            player_info,
            bgdef,
            text,
        })
    }
}

/// Resolves a path from a DEF `[Files]` list. Windows backslashes are treated
/// as separators so content authored on Windows loads on Unix.
pub(crate) fn resolve_path(dir: &Path, raw: &str) -> PathBuf {
    dir.join(raw.trim().replace('\\', "/"))
}

pub fn section_map(section: &crate::text::Section) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for e in &section.entries {
        map.insert(
            e.key.trim().to_ascii_lowercase(),
            e.value.trim().to_string(),
        );
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "manato-def-{}-{name}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(name);
        std::fs::write(&path, body).expect("write def");
        path
    }

    #[test]
    fn parses_character_metadata_files_and_localcoord() {
        let path = temp_file(
            "char.def",
            "[Info]\nname = Test\ndisplayname = Test Fighter\nauthor = Someone\n\
             mugenversion = 1.1\nlocalcoord = 640, 480\npal1 = char.act\n\
             [Files]\ncmd = char.cmd\ncns = char.cns\nst = char.cns\n\
             st1 = extra1.st, extra2.st\nst2 = extra3.st\n\
             stcommon = common1.cns, common2.cns\nsprite = char.sff\n\
             anim = char.air\nsound = char.snd\nai = char.ai\n\
             [Palette Keymap]\nx = 1\ny = 2\n",
        );
        let def = CharacterDef::load(&path).expect("parse def");
        assert_eq!(def.name, "Test");
        assert_eq!(def.display_name, "Test Fighter");
        assert_eq!(def.author, "Someone");
        assert_eq!(def.mugen_version, "1.1");
        assert_eq!(def.localcoord, (640.0, 480.0));
        assert_eq!(def.info.get("pal1").map(String::as_str), Some("char.act"));
        assert_eq!(def.palette_keymap.get("x"), Some(&1));
        assert_eq!(def.palette_keymap.get("y"), Some(&2));
        assert!(def.files.cmd.as_ref().unwrap().ends_with("char.cmd"));
        assert_eq!(def.files.states.len(), 4, "st, st1 (two files), st2");
        assert_eq!(def.files.common_states.len(), 2);
        assert!(def.files.ai.as_ref().unwrap().ends_with("char.ai"));
    }

    #[test]
    fn state_files_use_natural_order_beyond_st9() {
        let path = temp_file(
            "order.def",
            "[Info]\nname = Order\n[Files]\nst = base.cns\n\
             st10 = ten.cns\nst2 = two.cns\nst1 = one.cns\n\
             st0 = zero.cns\nstcommon = common.cns\nstuff = ignored.cns\n",
        );
        let def = CharacterDef::load(&path).expect("parse def");
        let names: Vec<String> = def
            .files
            .states
            .iter()
            .map(|path| {
                path.file_name()
                    .expect("file name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            names,
            ["base.cns", "zero.cns", "one.cns", "two.cns", "ten.cns"],
            "st, st0, st1, st2, st10 in natural order"
        );
        assert_eq!(def.files.common_states.len(), 1);
        assert_eq!(
            def.files.common_states[0]
                .file_name()
                .expect("name")
                .to_string_lossy(),
            "common.cns"
        );
    }

    #[test]
    fn normalizes_backslash_file_paths() {
        let path = temp_file(
            "win.def",
            "[Info]\nname = Win\n[Files]\ncmd = sub\\win.cmd\n\
             cns = sub\\win.cns\nst = sub\\base.cns\nst1 = sub\\extra.cns\n\
             stcommon = sub\\common.cns\nsprite = sub\\win.sff\n\
             anim = sub\\win.air\nsound = sub\\win.snd\nai = sub\\win.ai\n",
        );
        let def = CharacterDef::load(&path).expect("parse def");
        let check = |path: &Option<PathBuf>, expected: &str| {
            assert!(
                path.as_ref().expect("path").ends_with(expected),
                "expected {expected}, got {:?}",
                path
            );
        };
        check(&def.files.cmd, "sub/win.cmd");
        check(&def.files.cns, "sub/win.cns");
        check(&def.files.sprite, "sub/win.sff");
        check(&def.files.anim, "sub/win.air");
        check(&def.files.sound, "sub/win.snd");
        check(&def.files.ai, "sub/win.ai");
        assert!(def.files.states[0].ends_with("sub/base.cns"));
        assert!(def.files.states[1].ends_with("sub/extra.cns"));
        assert!(def.files.common_states[0].ends_with("sub/common.cns"));
    }

    #[test]
    fn parses_stage_metadata_and_localcoord() {
        let path = temp_file(
            "stage.def",
            "[Info]\nname = Room\ndisplayname = Test Room\nauthor = Someone\n\
             mugenversion = 1.1\nlocalcoord = 640, 360\n\
             [Camera]\nstartx = 10\nstarty = 0\nboundleft = -100\n\
             [PlayerInfo]\np1startx = -40\np2startx = 40\n\
             [BGdef]\nspr = stage.sff\n",
        );
        let def = StageDefFile::load(&path).expect("parse stage def");
        assert_eq!(def.name, "Room");
        assert_eq!(def.display_name, "Test Room");
        assert_eq!(def.mugen_version, "1.1");
        assert_eq!(def.localcoord, (640.0, 360.0));
        assert_eq!(def.camera.get("startx").map(String::as_str), Some("10"));
        assert_eq!(
            def.player_info.get("p1startx").map(String::as_str),
            Some("-40")
        );
        assert_eq!(def.bgdef.get("spr").map(String::as_str), Some("stage.sff"));
    }
}
