//! A content mod's `assets/` and `data/` (API 0.2, docs/modpacks.md): the
//! files it adds to the game's resource tree, and the blocks and items it
//! declares in `data/<id>/horizon/block/` and `data/<id>/horizon/item/`.
//!
//! Content is data, not code: nothing here runs in the sandbox. What a mod
//! may add is fenced instead. Its own namespaces (its id and the ones its
//! manifest lists), plus additions to vanilla tags and to the convention
//! tags (`c`, which NeoForge and Fabric mods share). Paths are plain, files
//! and totals are capped, vanilla ids cannot be redefined, and no two mods
//! share a namespace ([`clash`]).
use crate::manifest::{Capability, MANIFEST, Manifest};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
};

/// Content files per mod, bytes per file and bytes per mod.
pub const MAX_FILES: usize = 65_536;
pub const MAX_FILE: u64 = 16 << 20;
pub const MAX_TOTAL: u64 = 512 << 20;

/// Checks a content path: `assets/` or `data/`, then `/`-separated parts of
/// `[a-z0-9_.-]`, none of them `.` or `..` (vanilla's resource-location
/// characters, so the path means the same on every OS).
pub fn check_path(name: &str) -> Result<(), String> {
    let bad = || format!("{name:?} is not a plain assets/ or data/ path");
    let mut parts = name.split('/');
    if !matches!(parts.next(), Some("assets" | "data")) {
        return Err(bad());
    }
    let mut count = 0;
    for part in parts {
        count += 1;
        let chars_ok = part
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b));
        if part.is_empty() || part == "." || part == ".." || !chars_ok {
            return Err(bad());
        }
    }
    // assets/<namespace>/<file> at least.
    if count < 2 {
        return Err(bad());
    }
    Ok(())
}

/// A vanilla identifier, `minecraft:path`.
fn vanilla_id(id: &str) -> bool {
    id.strip_prefix("minecraft:").is_some_and(id_path)
}

/// Any identifier, `namespace:path`.
fn any_id(id: &str) -> bool {
    id.split_once(':').is_some_and(|(ns, path)| {
        !ns.is_empty()
            && ns
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
            && id_path(path)
    })
}

fn id_path(path: &str) -> bool {
    !path.is_empty()
        && path
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_./-".contains(&b))
}

/// What a declared block changes from its template; unset keeps the
/// template's value.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BlockProperties {
    /// Destroy time in seconds of hand mining (-1: unbreakable).
    pub hardness: Option<f32>,
    pub explosion_resistance: Option<f32>,
    /// Light emitted, 0 to 15.
    pub light: Option<u8>,
    pub requires_tool: Option<bool>,
    pub friction: Option<f32>,
    /// A `MapColor` id, 0 to 61.
    pub map_color: Option<u8>,
    /// A vanilla block whose sounds this block uses.
    pub sounds: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockFile {
    template: String,
    #[serde(default)]
    properties: BlockProperties,
    #[serde(default)]
    item: ItemChoice,
}

/// A block file's `item`: whether the block has an item named as the block
/// (`true`, the default, or `false`), or the id of its item when that is
/// named otherwise, as a crop's seeds are (`"farmersdelight:cabbage_seeds"`
/// for `farmersdelight:cabbages`).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ItemChoice {
    Has(bool),
    Named(String),
}
impl Default for ItemChoice {
    fn default() -> Self {
        ItemChoice::Has(true)
    }
}

/// A block the mod adds: a vanilla template's behaviour, states and shapes
/// under a new id, with `properties` applied.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// `<mod id>:<path>`.
    pub id: String,
    /// The vanilla block it copies, `minecraft:<path>`.
    pub template: String,
    pub properties: BlockProperties,
    /// The block's item: `<mod id>:<path>`, the block's own id unless the
    /// file names another; `None` for a block without one. An item declared
    /// under that id in `item/` is the block's item; otherwise the item is a
    /// copy of the template block's.
    pub item: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Epic,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemFile {
    template: String,
    max_stack_size: Option<u8>,
    rarity: Option<Rarity>,
    /// Default components in the 26.3 format, as a data pack's item
    /// modifier writes them: `"minecraft:food": {...}` sets one,
    /// `"!minecraft:food": {}` removes the template's.
    #[serde(default)]
    components: BTreeMap<String, serde_json::Value>,
    crafting_remainder: Option<String>,
}

/// An item the mod adds: a vanilla template item's components under a new
/// id, with these changed.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    /// The vanilla item it copies, `minecraft:<path>`.
    pub template: String,
    pub max_stack_size: Option<u8>,
    pub rarity: Option<Rarity>,
    /// Components set (`Some`) or removed (`None`) by name,
    /// `minecraft:<component>`. The game checks the names against its
    /// component registry.
    pub components: BTreeMap<String, Option<serde_json::Value>>,
    /// What stays in the crafting grid when a recipe uses the item
    /// (`Item.Properties.craftRemainder`, not a component in 26.3): the
    /// bowl of a stew, the bottle of a drink.
    pub crafting_remainder: Option<String>,
}

#[derive(Debug)]
enum Source {
    Dir(PathBuf),
    Zip(PathBuf),
}

/// A mod's content: its files and declarations.
#[derive(Debug)]
pub struct Content {
    source: Source,
    /// Every content file (`assets/...`, `data/...`), sorted.
    pub files: Vec<String>,
    pub blocks: Vec<Block>,
    pub items: Vec<Item>,
    /// The namespaces it owns: the mod id, then the manifest's `namespaces`.
    pub namespaces: Vec<String>,
}

/// Why two content mods cannot be on together: a namespace both own (one's
/// id or listed namespace is the other's), which would mix their files.
/// `a` and `b` are (mod id, owned namespaces).
pub fn clash(a: (&str, &[String]), b: (&str, &[String])) -> Option<String> {
    let shared = a.1.iter().find(|ns| b.1.contains(ns))?;
    Some(format!(
        "mods {} and {} both use the namespace {shared}; turn one of them off",
        a.0, b.0
    ))
}

impl Content {
    /// The content of the mod at `path` (a `.hmod` or a mod folder).
    pub fn read(path: &Path, manifest: &Manifest) -> Result<Content, String> {
        let (source, sized) = if path.is_dir() {
            let mut out = Vec::new();
            for top in ["assets", "data"] {
                walk(path, &path.join(top), &mut out)?;
            }
            (Source::Dir(path.to_owned()), out)
        } else {
            let mut zip = open_zip(path)?;
            let mut out = Vec::new();
            for i in 0..zip.len() {
                let entry = zip.by_index_raw(i).map_err(|e| e.to_string())?;
                let name = entry.name();
                if (name.starts_with("assets/") || name.starts_with("data/")) && !entry.is_dir() {
                    out.push((name.to_owned(), entry.size()));
                }
            }
            (Source::Zip(path.to_owned()), out)
        };
        let mut files = Vec::with_capacity(sized.len());
        let mut total = 0u64;
        for (name, size) in sized {
            check_path(&name)?;
            if size > MAX_FILE {
                return Err(format!("{name} is over {} MiB", MAX_FILE >> 20));
            }
            total += size;
            files.push(name);
        }
        if files.len() > MAX_FILES {
            return Err(format!("more than {MAX_FILES} content files"));
        }
        if total > MAX_TOTAL {
            return Err(format!("content is over {} MiB", MAX_TOTAL >> 20));
        }
        files.sort();
        files.dedup();
        let mut content = Content {
            source,
            files,
            blocks: Vec::new(),
            items: Vec::new(),
            namespaces: manifest.owned_namespaces().map(str::to_owned).collect(),
        };
        content.check(manifest)?;
        Ok(content)
    }

    /// Where the files are read from: the mod's folder or its `.hmod`.
    pub fn source(&self) -> &Path {
        match &self.source {
            Source::Dir(path) | Source::Zip(path) => path,
        }
    }

    /// No files: a mod with code only.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// One content file's bytes.
    pub fn file(&self, name: &str) -> Result<Vec<u8>, String> {
        check_path(name)?;
        let mut out = Vec::new();
        match &self.source {
            Source::Dir(root) => {
                let file =
                    std::fs::File::open(root.join(name)).map_err(|e| format!("{name}: {e}"))?;
                file.take(MAX_FILE + 1)
                    .read_to_end(&mut out)
                    .map_err(|e| format!("{name}: {e}"))?;
            }
            Source::Zip(path) => {
                let mut zip = open_zip(path)?;
                let entry = zip.by_name(name).map_err(|e| format!("{name}: {e}"))?;
                entry
                    .take(MAX_FILE + 1)
                    .read_to_end(&mut out)
                    .map_err(|e| format!("{name}: {e}"))?;
            }
        }
        if out.len() as u64 > MAX_FILE {
            return Err(format!("{name} is over {} MiB", MAX_FILE >> 20));
        }
        Ok(out)
    }

    /// Where content may go, and the declarations.
    fn check(&mut self, manifest: &Manifest) -> Result<(), String> {
        let id = manifest.id.as_str();
        if !self.files.is_empty() && !manifest.capabilities.contains(&Capability::Registries) {
            return Err(format!(
                "has assets/ or data/ but not capability registries in {MANIFEST}"
            ));
        }
        let mut blocks = Vec::new();
        let mut items = Vec::new();
        for name in &self.files {
            let mut parts = name.splitn(3, '/');
            let (top, namespace, rest) = (
                parts.next().unwrap_or(""),
                parts.next().unwrap_or(""),
                parts.next().unwrap_or(""),
            );
            let own = namespace == id;
            let owned = own || manifest.namespaces.iter().any(|n| n == namespace);
            // Vanilla's own files stay vanilla's; a mod may only add to its
            // tags, and to the convention tags mods share.
            let shared = matches!(namespace, "minecraft" | "c");
            let allowed = owned || (top == "data" && shared && rest.starts_with("tags/"));
            if !allowed {
                return Err(format!(
                    "{name}: a mod adds files in its own namespaces ({top}/{id}/ and those listed in namespaces) or to vanilla and convention (c) tags"
                ));
            }
            if !own && top == "data" && rest.starts_with("horizon/") {
                return Err(format!(
                    "{name}: blocks and items are declared in data/{id}/horizon/"
                ));
            }
            let Some(decl) = (own && top == "data")
                .then(|| rest.strip_prefix("horizon/"))
                .flatten()
            else {
                continue;
            };
            let parse = |kind: &str| -> Option<String> {
                let path = decl
                    .strip_prefix(kind)?
                    .strip_prefix('/')?
                    .strip_suffix(".json")?;
                Some(format!("{id}:{path}"))
            };
            if let Some(block) = parse("block") {
                blocks.push((block, name.clone()));
            } else if let Some(item) = parse("item") {
                items.push((item, name.clone()));
            } else {
                return Err(format!(
                    "{name}: data/{id}/horizon/ holds block/*.json and item/*.json only"
                ));
            }
        }
        // Items so far, and the ones blocks name as theirs: an item file
        // may declare those.
        let mut ids = BTreeSet::new();
        let mut named = BTreeSet::new();
        for (block, name) in blocks {
            let file: BlockFile =
                serde_json::from_slice(&self.file(&name)?).map_err(|e| format!("{name}: {e}"))?;
            check_block(&file).map_err(|e| format!("{name}: {e}"))?;
            let item = match file.item {
                ItemChoice::Has(false) => None,
                ItemChoice::Has(true) => Some(block.clone()),
                ItemChoice::Named(item) => {
                    if !item.strip_prefix(&format!("{id}:")).is_some_and(id_path) {
                        return Err(format!(
                            "{name}: item {item:?} is not an item id of this mod ({id}:...)"
                        ));
                    }
                    named.insert(item.clone());
                    Some(item)
                }
            };
            if let Some(item) = &item
                && !ids.insert(item.clone())
            {
                return Err(format!("{name}: {item} is another block's item too"));
            }
            self.blocks.push(Block {
                id: block,
                template: file.template,
                properties: file.properties,
                item,
            });
        }
        for (item, name) in items {
            let file: ItemFile =
                serde_json::from_slice(&self.file(&name)?).map_err(|e| format!("{name}: {e}"))?;
            if !vanilla_id(&file.template) {
                return Err(format!(
                    "{name}: template {:?} is not a vanilla item (minecraft:...)",
                    file.template
                ));
            }
            if file.max_stack_size.is_some_and(|n| !(1..=99).contains(&n)) {
                return Err(format!("{name}: max_stack_size is 1 to 99"));
            }
            let components = components(file.components).map_err(|e| format!("{name}: {e}"))?;
            for (field, set, key) in [
                (
                    "max_stack_size",
                    file.max_stack_size.is_some(),
                    "minecraft:max_stack_size",
                ),
                ("rarity", file.rarity.is_some(), "minecraft:rarity"),
            ] {
                if set && components.contains_key(key) {
                    return Err(format!("{name}: {field} is set twice (also in components)"));
                }
            }
            if let Some(r) = &file.crafting_remainder
                && !any_id(r)
            {
                return Err(format!(
                    "{name}: crafting_remainder {r:?} is not an item id (namespace:path)"
                ));
            }
            if !ids.insert(item.clone()) && !named.remove(&item) {
                return Err(format!("{name}: {item} is also a block's item"));
            }
            self.items.push(Item {
                id: item,
                template: file.template,
                max_stack_size: file.max_stack_size,
                rarity: file.rarity,
                components,
                crafting_remainder: file.crafting_remainder,
            });
        }
        Ok(())
    }
}

/// An item file's `components`: vanilla names, each set or removed once.
fn components(
    raw: BTreeMap<String, serde_json::Value>,
) -> Result<BTreeMap<String, Option<serde_json::Value>>, String> {
    let mut out = BTreeMap::new();
    for (key, value) in raw {
        let (name, value) = match key.strip_prefix('!') {
            Some(name) => (name.to_owned(), None),
            None => (key, Some(value)),
        };
        if !vanilla_id(&name) {
            return Err(format!(
                "component {name:?} is not a vanilla component (minecraft:...)"
            ));
        }
        if out.insert(name.clone(), value).is_some() {
            return Err(format!("component {name} is both set and removed"));
        }
    }
    Ok(out)
}

fn check_block(file: &BlockFile) -> Result<(), String> {
    if !vanilla_id(&file.template) {
        return Err(format!(
            "template {:?} is not a vanilla block (minecraft:...)",
            file.template
        ));
    }
    let p = &file.properties;
    let finite = |v: Option<f32>| v.is_none_or(f32::is_finite);
    if !finite(p.hardness) || !finite(p.explosion_resistance) || !finite(p.friction) {
        return Err("hardness, explosion_resistance and friction are numbers".into());
    }
    if p.hardness.is_some_and(|h| h < 0.0 && h != -1.0) {
        return Err("hardness is 0 or more, or -1 (unbreakable)".into());
    }
    if p.explosion_resistance.is_some_and(|r| r < 0.0) {
        return Err("explosion_resistance is 0 or more".into());
    }
    if p.friction.is_some_and(|f| !(0.0..=2.0).contains(&f)) {
        return Err("friction is 0 to 2".into());
    }
    if p.light.is_some_and(|l| l > 15) {
        return Err("light is 0 to 15".into());
    }
    if p.map_color.is_some_and(|c| c > 61) {
        return Err("map_color is a MapColor id, 0 to 61".into());
    }
    if p.sounds.as_deref().is_some_and(|s| !vanilla_id(s)) {
        return Err("sounds names a vanilla block (minecraft:...)".into());
    }
    Ok(())
}

fn open_zip(path: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    zip::ZipArchive::new(file).map_err(|e| e.to_string())
}

/// Files under `dir`, relative to `root` with `/` separators. Symbolic links
/// are not followed: a mod folder holds its own files.
fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>) -> Result<(), String> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in read.flatten() {
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.is_dir() {
            walk(root, &path, out)?;
        } else if meta.is_file() {
            let rel = path.strip_prefix(root).map_err(|e| e.to_string())?;
            let parts: Option<Vec<&str>> = rel.iter().map(|p| p.to_str()).collect();
            let name = parts.ok_or_else(|| format!("{} is not UTF-8", rel.display()))?;
            out.push((name.join("/"), meta.len()));
        }
        if out.len() > MAX_FILES {
            return Err(format!("more than {MAX_FILES} content files"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Package;

    const MANIFEST_TEXT: &str = r#"
id = "ruby"
name = "Ruby"
version = "0.1.0"
api = "0.2"
side = "both"
capabilities = ["registries"]
"#;
    const BLOCK: &[u8] = br#"{"template": "minecraft:iron_block",
        "properties": {"hardness": 4.0, "light": 3, "sounds": "minecraft:amethyst_block"}}"#;
    const ITEM: &[u8] = br#"{"template": "minecraft:diamond", "rarity": "uncommon"}"#;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-content-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn hmod(name: &str, manifest: &str, files: &[(&str, &[u8])]) -> Result<Content, String> {
        let dir = scratch(name);
        let path = dir.join("m.hmod");
        std::fs::write(&path, Package::write_files(manifest, None, files)?).unwrap();
        let package = Package::read(&path);
        let content = package.and_then(|p| Content::read(&path, &p.manifest));
        std::fs::remove_dir_all(&dir).unwrap();
        content
    }

    #[test]
    fn content_only_mod_round_trips() {
        let files: &[(&str, &[u8])] = &[
            ("data/ruby/horizon/block/ruby_block.json", BLOCK),
            ("data/ruby/horizon/item/ruby.json", ITEM),
            ("data/ruby/recipe/ruby_block.json", b"{}"),
            ("data/minecraft/tags/block/mineable/pickaxe.json", b"{}"),
            ("data/c/tags/item/gems/ruby.json", b"{}"),
            ("assets/ruby/textures/block/ruby_block.png", b"png"),
        ];
        let content = hmod("ok", MANIFEST_TEXT, files).unwrap();
        assert_eq!(content.files.len(), 6);
        assert_eq!(
            content.blocks,
            [Block {
                id: "ruby:ruby_block".into(),
                template: "minecraft:iron_block".into(),
                properties: BlockProperties {
                    hardness: Some(4.0),
                    light: Some(3),
                    sounds: Some("minecraft:amethyst_block".into()),
                    ..BlockProperties::default()
                },
                item: Some("ruby:ruby_block".into()),
            }]
        );
        assert_eq!(content.items[0].id, "ruby:ruby");
        assert_eq!(content.items[0].rarity, Some(Rarity::Uncommon));
    }

    #[test]
    fn folder_mod_reads_the_same() {
        let dir = scratch("folder");
        std::fs::write(dir.join(MANIFEST), MANIFEST_TEXT).unwrap();
        let block = dir.join("data/ruby/horizon/block");
        std::fs::create_dir_all(&block).unwrap();
        std::fs::write(block.join("ruby_block.json"), BLOCK).unwrap();
        let package = Package::read(&dir).unwrap();
        assert!(package.wasm.is_none());
        let content = Content::read(&dir, &package.manifest).unwrap();
        assert_eq!(content.files, ["data/ruby/horizon/block/ruby_block.json"]);
        assert_eq!(content.file(&content.files[0]).unwrap(), BLOCK);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_content_outside_its_fence() {
        let refused = |name: &str, files: &[(&str, &[u8])]| {
            let e = hmod(name, MANIFEST_TEXT, files).unwrap_err();
            assert!(!e.is_empty());
            e
        };
        let e = refused("other-ns", &[("data/other/horizon/block/a.json", BLOCK)]);
        assert!(e.contains("own namespaces"), "{e}");
        refused(
            "vanilla-asset",
            &[("assets/minecraft/textures/block/stone.png", b"x")],
        );
        refused(
            "vanilla-recipe",
            &[("data/minecraft/recipe/stone.json", b"{}")],
        );
        refused("convention-recipe", &[("data/c/recipe/stone.json", b"{}")]);
        refused("convention-asset", &[("assets/c/lang/en_us.json", b"{}")]);
        let e = refused(
            "modded-template",
            &[(
                "data/ruby/horizon/block/a.json",
                br#"{"template": "ruby:other"}"#,
            )],
        );
        assert!(e.contains("not a vanilla block"), "{e}");
        refused(
            "light",
            &[(
                "data/ruby/horizon/block/a.json",
                br#"{"template": "minecraft:stone", "properties": {"light": 16}}"#,
            )],
        );
        refused(
            "unknown-key",
            &[(
                "data/ruby/horizon/block/a.json",
                br#"{"template": "minecraft:stone", "glow": true}"#,
            )],
        );
        let e = refused(
            "clash",
            &[
                ("data/ruby/horizon/block/a.json", BLOCK),
                ("data/ruby/horizon/item/a.json", ITEM),
            ],
        );
        assert!(e.contains("also a block's item"), "{e}");
        refused("other-kind", &[("data/ruby/horizon/entity/a.json", b"{}")]);
        refused(
            "stack",
            &[(
                "data/ruby/horizon/item/a.json",
                br#"{"template": "minecraft:stick", "max_stack_size": 100}"#,
            )],
        );
    }

    #[test]
    fn items_declare_components_and_remainders() {
        let stew: &[u8] = br#"{"template": "minecraft:mushroom_stew",
            "components": {
                "minecraft:food": {"nutrition": 10, "saturation": 12.0},
                "!minecraft:use_remainder": {}
            },
            "crafting_remainder": "minecraft:bowl"}"#;
        let content = hmod(
            "components",
            MANIFEST_TEXT,
            &[("data/ruby/horizon/item/stew.json", stew)],
        )
        .unwrap();
        let item = &content.items[0];
        assert_eq!(
            item.components["minecraft:food"],
            Some(serde_json::json!({"nutrition": 10, "saturation": 12.0}))
        );
        assert_eq!(item.components["minecraft:use_remainder"], None);
        assert_eq!(item.crafting_remainder.as_deref(), Some("minecraft:bowl"));

        let refused = |name: &str, file: &[u8]| {
            hmod(
                name,
                MANIFEST_TEXT,
                &[("data/ruby/horizon/item/a.json", file)],
            )
            .unwrap_err()
        };
        let e = refused(
            "modded-component",
            br#"{"template": "minecraft:bread", "components": {"ruby:glow": {}}}"#,
        );
        assert!(e.contains("not a vanilla component"), "{e}");
        let e = refused(
            "set-and-removed",
            br#"{"template": "minecraft:bread",
                "components": {"minecraft:food": {}, "!minecraft:food": {}}}"#,
        );
        assert!(e.contains("both set and removed"), "{e}");
        let e = refused(
            "twice",
            br#"{"template": "minecraft:bread", "rarity": "rare",
                "components": {"minecraft:rarity": "epic"}}"#,
        );
        assert!(e.contains("set twice"), "{e}");
        let e = refused(
            "remainder",
            br#"{"template": "minecraft:bread", "crafting_remainder": "Bowl"}"#,
        );
        assert!(e.contains("not an item id"), "{e}");
    }

    #[test]
    fn blocks_name_their_items() {
        let crop: &[u8] = br#"{"template": "minecraft:wheat", "item": "ruby:ruby_seeds"}"#;
        let seeds: &[u8] = br#"{"template": "minecraft:wheat_seeds", "rarity": "uncommon"}"#;
        let content = hmod(
            "named",
            MANIFEST_TEXT,
            &[
                ("data/ruby/horizon/block/rubies.json", crop),
                ("data/ruby/horizon/item/ruby_seeds.json", seeds),
                (
                    "data/ruby/horizon/block/ruby_glass.json",
                    br#"{"template": "minecraft:glass", "item": false}"#,
                ),
            ],
        )
        .unwrap();
        let item = |id: &str| {
            content
                .blocks
                .iter()
                .find(|b| b.id == id)
                .unwrap()
                .item
                .clone()
        };
        assert_eq!(item("ruby:rubies").as_deref(), Some("ruby:ruby_seeds"));
        assert_eq!(item("ruby:ruby_glass"), None);
        assert_eq!(content.items[0].id, "ruby:ruby_seeds");
        // Without an item file, the game copies the template's item.
        let content = hmod(
            "named-alone",
            MANIFEST_TEXT,
            &[("data/ruby/horizon/block/rubies.json", crop)],
        )
        .unwrap();
        assert!(content.items.is_empty());

        let e = hmod(
            "named-other-ns",
            MANIFEST_TEXT,
            &[(
                "data/ruby/horizon/block/rubies.json",
                br#"{"template": "minecraft:wheat", "item": "minecraft:wheat_seeds"}"#,
            )],
        )
        .unwrap_err();
        assert!(e.contains("not an item id of this mod"), "{e}");
        let e = hmod(
            "named-twice",
            MANIFEST_TEXT,
            &[
                ("data/ruby/horizon/block/a.json", crop),
                ("data/ruby/horizon/block/b.json", crop),
            ],
        )
        .unwrap_err();
        assert!(e.contains("another block's item"), "{e}");
    }

    #[test]
    fn listed_namespaces_hold_content() {
        let manifest = format!("{MANIFEST_TEXT}namespaces = [\"kaisyn\"]\n");
        let files: &[(&str, &[u8])] = &[
            ("data/kaisyn/worldgen/template_pool/a.json", b"{}"),
            ("data/kaisyn/structure/a.nbt", b"nbt"),
            ("assets/kaisyn/lang/en_us.json", b"{}"),
        ];
        let content = hmod("listed", &manifest, files).unwrap();
        assert_eq!(content.files.len(), 3);
        assert_eq!(content.namespaces, ["ruby", "kaisyn"]);
        // Declarations stay in the id's namespace.
        let decl: &[(&str, &[u8])] = &[("data/kaisyn/horizon/block/a.json", BLOCK)];
        let e = hmod("listed-decl", &manifest, decl).unwrap_err();
        assert!(e.contains("declared in data/ruby/horizon/"), "{e}");
        // A namespace it does not list is still outside.
        let other: &[(&str, &[u8])] = &[("data/other/structure/a.nbt", b"nbt")];
        assert!(hmod("listed-other", &manifest, other).is_err());
    }

    #[test]
    fn mods_sharing_a_namespace_clash() {
        let t_and_t = ["t_and_t".to_owned(), "kaisyn".to_owned()];
        let kaisyn = ["kaisyn".to_owned()];
        let ruby = ["ruby".to_owned(), "gems".to_owned()];
        assert_eq!(
            clash(("t_and_t", &t_and_t), ("kaisyn", &kaisyn)).as_deref(),
            Some("mods t_and_t and kaisyn both use the namespace kaisyn; turn one of them off")
        );
        assert_eq!(clash(("t_and_t", &t_and_t), ("ruby", &ruby)), None);
    }

    #[test]
    fn content_needs_the_registries_capability() {
        let code_only = MANIFEST_TEXT
            .replace("api = \"0.2\"", "api = \"0.1\"")
            .replace("capabilities = [\"registries\"]", "capabilities = []");
        // No code and no registries: nothing to load.
        let e = hmod("empty", &code_only, &[]).unwrap_err();
        assert!(e.contains("mod.wasm: missing"), "{e}");
        // Registries on API 0.1.
        let old = MANIFEST_TEXT.replace("api = \"0.2\"", "api = \"0.1\"");
        assert!(Manifest::parse(&old).unwrap_err().contains("needs api"));
        // Content without the capability, in a mod with code.
        let dir = scratch("no-cap");
        let path = dir.join("m.hmod");
        let files: &[(&str, &[u8])] = &[("assets/ruby/lang/en_us.json", b"{}")];
        std::fs::write(
            &path,
            Package::write_files(&code_only, Some(b"\0asm"), files).unwrap(),
        )
        .unwrap();
        let package = Package::read(&path).unwrap();
        let e = Content::read(&path, &package.manifest).err().unwrap();
        assert!(e.contains("not capability registries"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn paths_are_plain() {
        assert!(check_path("assets/ruby/textures/block/ruby_block.png").is_ok());
        assert!(check_path("data/ruby/loot_table/blocks/a.json").is_ok());
        for bad in [
            "assets/ruby",
            "assets/../x/y",
            "assets/ruby//a",
            "assets/Ruby/a",
            "assets/ruby/a b",
            "/assets/ruby/a",
            "other/ruby/a",
            "assets\\ruby\\a",
        ] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
    }
}
