use std::collections::HashMap;
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
}

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

        let resolve = |key: &str| files_section.get(key).map(|v| dir.join(v.trim()));
        let mut states = Vec::new();
        for key in [
            "st", "st1", "st2", "st3", "st4", "st5", "st6", "st7", "st8", "st9",
        ] {
            if let Some(v) = files_section.get(key) {
                for part in v.split(',') {
                    let p = dir.join(part.trim());
                    if !p.as_os_str().is_empty() {
                        states.push(p);
                    }
                }
            }
        }
        let mut common_states = Vec::new();
        if let Some(v) = files_section.get("stcommon") {
            for part in v.split(',') {
                let p = dir.join(part.trim());
                if !p.as_os_str().is_empty() {
                    common_states.push(p);
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
        };

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
