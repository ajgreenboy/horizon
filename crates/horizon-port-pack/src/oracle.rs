//! The NeoForge oracle's facts (`tools/neoforge_oracle.py export`, in
//! `<work>/oracle/facts.json`): every block, item, recipe and tag the pack's
//! mods register, read from the running game, and the vanilla blocks beside
//! them. Carry-over uses them in place of guesses: a block's template is the
//! vanilla block of the same class and states whose shapes and sounds match
//! best, and the declared properties are the mod's own numbers.
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Deserialize)]
pub struct Facts {
    pub blocks: BTreeMap<String, Block>,
    pub items: BTreeMap<String, Item>,
    #[serde(default)]
    pub tags: Tags,
}

#[derive(Debug, Default, Deserialize)]
pub struct Tags {
    #[serde(default)]
    pub block: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub item: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct Block {
    /// The class and its superclasses, most derived first.
    pub class: Vec<String>,
    pub properties: BTreeMap<String, Vec<String>>,
    pub default: String,
    pub item: String,
    pub explosion_resistance: f32,
    pub friction: f32,
    pub speed_factor: f32,
    pub jump_factor: f32,
    #[serde(default)]
    pub block_set_type: Option<SetType>,
    #[serde(default)]
    pub wood_type: Option<String>,
    pub states: BTreeMap<String, State>,
}

#[derive(Debug, Deserialize)]
pub struct SetType {
    pub name: String,
    pub can_open_by_hand: bool,
}

#[derive(Debug, Deserialize)]
pub struct State {
    pub hardness: f32,
    pub emission: u8,
    pub requires_tool: bool,
    pub map_color_id: u8,
    pub sound: Value,
    pub outline: Value,
    pub collision: Value,
    /// What breaking the state drops, per tool (`hand`, `diamond_pickaxe`,
    /// `shears`, `silk_touch_pickaxe`...): item id → count.
    #[serde(default)]
    pub drops: BTreeMap<String, BTreeMap<String, u32>>,
}

#[derive(Debug, Deserialize)]
pub struct Item {
    pub class: String,
    pub max_stack_size: u8,
    pub rarity: String,
    #[serde(default)]
    pub block: Option<String>,
    /// `nutrition`, `saturation`, `can_always_eat`, `eat_seconds`,
    /// `using_converts_to` and `effects` (`id`, `duration`, `amplifier`,
    /// `probability`), as 1.21.1's `FoodProperties`.
    #[serde(default)]
    pub food: Option<Value>,
    /// `UseAnim`, lower case (`eat`, `drink`, `none`...).
    #[serde(default)]
    pub use_animation: Option<String>,
    #[serde(default)]
    pub crafting_remainder: Option<String>,
}

const VANILLA_PACKAGE: &str = "net.minecraft.";

/// A state key with its properties sorted, so keys compare across blocks.
/// Which states of a block drop the block itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelfDrop {
    /// Every state: `dropSelf`.
    Always,
    /// Only the states where this property has this value.
    When(String, String),
}

/// A state key's `property=value` pairs.
fn pairs(key: &str) -> BTreeSet<(String, String)> {
    key.split(',')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

pub fn state_key(key: &str) -> String {
    let mut parts: Vec<&str> = key.split(',').filter(|p| !p.is_empty()).collect();
    parts.sort_unstable();
    parts.join(",")
}

impl Block {
    /// The first vanilla class in the chain.
    pub fn vanilla_class(&self) -> &str {
        self.class
            .iter()
            .find(|c| c.starts_with(VANILLA_PACKAGE))
            .map_or("", String::as_str)
    }
    /// Its own class is a mod class: behaviour code the template may not have.
    pub fn custom_class(&self) -> bool {
        self.class
            .first()
            .is_some_and(|c| !c.starts_with(VANILLA_PACKAGE))
    }
    /// Every state drops one `item` (the block's own) with every tool, except
    /// where the state needs a tool and gets none: vanilla's `dropSelf`. Or
    /// only the states where one property has one value do, and the others
    /// drop nothing with any tool: a door's lower half (`createDoorTable`).
    /// Mods often register these tables in code, so their jar has no file.
    pub fn drops_itself(&self, item: &str) -> Option<SelfDrop> {
        let one = BTreeMap::from([(item.to_owned(), 1)]);
        let (mut dropping, mut nothing) = (Vec::new(), Vec::new());
        for (key, state) in &self.states {
            if state.drops.is_empty() {
                continue;
            }
            let mut drops = false;
            let mut short = false;
            for got in state.drops.values() {
                if *got == one {
                    drops = true;
                } else if got.is_empty() {
                    short = true;
                } else {
                    return None;
                }
            }
            match (drops, short) {
                (true, true) if !state.requires_tool => return None,
                (true, _) => dropping.push(pairs(key)),
                (false, _) => nothing.push(pairs(key)),
            }
        }
        let (first, rest) = dropping.split_first()?;
        if nothing.is_empty() {
            return Some(SelfDrop::Always);
        }
        first
            .iter()
            .filter(|pair| rest.iter().all(|s| s.contains(*pair)))
            .find(|pair| nothing.iter().all(|s| !s.contains(*pair)))
            .map(|(p, v)| SelfDrop::When(p.clone(), v.clone()))
    }
    fn default_state(&self) -> Option<&State> {
        self.states
            .get(&self.default)
            .or_else(|| self.states.values().next())
    }
    fn same_states(&self, other: &Block) -> bool {
        self.properties.len() == other.properties.len()
            && self.properties.iter().all(|(k, v)| {
                other.properties.get(k).is_some_and(|w| {
                    v.iter().collect::<BTreeSet<_>>() == w.iter().collect::<BTreeSet<_>>()
                })
            })
    }
    fn by_key(&self) -> BTreeMap<String, &State> {
        self.states.iter().map(|(k, s)| (state_key(k), s)).collect()
    }
}

/// The template carry-over gives a mod block.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub template: String,
    /// `data/<mod>/horizon/block/<name>.json`'s `properties`.
    pub properties: Map<String, Value>,
    /// What the template does not reproduce, for the report.
    pub differences: Vec<String>,
}

impl Facts {
    pub fn read(path: &Path) -> Result<Facts, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The vanilla block that best stands in for `id`: the same vanilla
    /// class and states, then the most states with the same outline and
    /// collision, then the same sounds, set type, wood type and hardness.
    /// `None` when no vanilla block has its class and states.
    pub fn template(&self, id: &str, allowed: &dyn Fn(&str) -> bool) -> Option<Choice> {
        let block = self.blocks.get(id)?;
        let class = block.vanilla_class();
        let own = block.by_key();
        let ours = block.default_state()?;
        let mut best: Option<(i64, &str, &Block)> = None;
        for (name, v) in &self.blocks {
            if !name.starts_with("minecraft:")
                || v.vanilla_class() != class
                || v.custom_class()
                || !block.same_states(v)
                || !allowed(name)
            {
                continue;
            }
            let theirs = v.by_key();
            let shapes = own
                .iter()
                .filter(|(k, s)| {
                    theirs
                        .get(*k)
                        .is_some_and(|t| t.outline == s.outline && t.collision == s.collision)
                })
                .count() as i64;
            let Some(td) = v.default_state() else {
                continue;
            };
            let same_set = match (&block.block_set_type, &v.block_set_type) {
                (Some(a), Some(b)) => {
                    (a.name == b.name) as i64 * 2
                        + (a.can_open_by_hand == b.can_open_by_hand) as i64 * 8
                }
                _ => 0,
            };
            let score = shapes * 1000
                + (td.sound == ours.sound) as i64 * 100
                + same_set * 10
                + (block.wood_type.is_some() && block.wood_type == v.wood_type) as i64 * 20
                + (td.hardness == ours.hardness) as i64 * 5
                + (td.map_color_id == ours.map_color_id) as i64;
            if best.is_none_or(|(s, n, _)| score > s || (score == s && name.as_str() < n)) {
                best = Some((score, name, v));
            }
        }
        let (_, template, t) = best?;
        Some(self.choice(block, template, t, allowed))
    }

    fn choice(
        &self,
        block: &Block,
        template: &str,
        t: &Block,
        allowed: &dyn Fn(&str) -> bool,
    ) -> Choice {
        let (ours, theirs) = (block.default_state(), t.default_state());
        let mut properties = Map::new();
        let mut differences = Vec::new();
        if let Some(s) = ours {
            properties.insert("hardness".into(), json!(s.hardness));
            properties.insert(
                "explosion_resistance".into(),
                json!(block.explosion_resistance),
            );
            properties.insert("light".into(), json!(s.emission));
            properties.insert("requires_tool".into(), json!(s.requires_tool));
            properties.insert("friction".into(), json!(block.friction));
            properties.insert("map_color".into(), json!(s.map_color_id));
            if theirs.is_none_or(|t| t.sound != s.sound) {
                match self.sound_block(&s.sound, allowed) {
                    Some(b) => {
                        properties.insert("sounds".into(), json!(b));
                    }
                    None => differences.push("sounds: no vanilla block has them".into()),
                }
            }
        }
        if (block.speed_factor, block.jump_factor) != (t.speed_factor, t.jump_factor) {
            differences.push(format!(
                "speed and jump factors {} {} (template {} {})",
                block.speed_factor, block.jump_factor, t.speed_factor, t.jump_factor
            ));
        }
        let theirs = t.by_key();
        let mut shapes = 0;
        let mut varying = BTreeSet::new();
        let first = ours.map(|s| (s.hardness, s.emission, s.requires_tool, s.map_color_id));
        for (k, s) in block.by_key() {
            let Some(v) = theirs.get(&k) else {
                continue;
            };
            if v.outline != s.outline || v.collision != s.collision {
                shapes += 1;
            }
            if Some((s.hardness, s.emission, s.requires_tool, s.map_color_id)) != first {
                varying.insert("per-state hardness, light or map colour");
            }
        }
        if shapes > 0 {
            differences.push(format!(
                "shapes differ from {template} in {shapes} of {} states",
                block.states.len()
            ));
        }
        differences.extend(varying.into_iter().map(str::to_owned));
        if block.custom_class() {
            differences.push(format!(
                "class {} (behaviour beyond {})",
                block.class[0],
                block.vanilla_class()
            ));
        }
        Choice {
            template: template.to_owned(),
            properties,
            differences,
        }
    }

    /// A vanilla block whose default state has these sounds.
    fn sound_block(&self, sound: &Value, allowed: &dyn Fn(&str) -> bool) -> Option<&str> {
        self.blocks
            .iter()
            .filter(|(n, _)| n.starts_with("minecraft:") && allowed(n))
            .find(|(_, b)| b.default_state().is_some_and(|s| &s.sound == sound))
            .map(|(n, _)| n.as_str())
    }

    /// An item declaration for a mod item that is not a declared block's:
    /// a vanilla template item (bread for food, else a stick) with its stack
    /// size, rarity, food and remainders, and what it does not reproduce.
    pub fn item(&self, id: &str) -> Result<(Value, Vec<String>), String> {
        let item = self.items.get(id).ok_or("not in the oracle")?;
        if item.block.is_some() {
            return Err("a block item without a block".into());
        }
        let template = if item.food.is_some() {
            "minecraft:bread"
        } else {
            "minecraft:stick"
        };
        Ok(self.declare(id, item, template))
    }

    /// The item a mod block names as its own when that has another id (a
    /// crop's seeds): one of the mod's items whose block item places this
    /// block, with a template that has an item to copy. A wall sign shares
    /// its standing sign's item and gets none.
    pub fn named_item(&self, id: &str, template: &str) -> Option<&str> {
        let block = self.blocks.get(id)?;
        let item = block.item.as_str();
        let own = id.split_once(':')?.0;
        (item != id
            && item.split_once(':').is_some_and(|(ns, _)| ns == own)
            && self.items.get(item)?.block.as_deref() == Some(id)
            && self
                .blocks
                .get(template)
                .is_some_and(|t| t.item != "minecraft:air"))
        .then_some(item)
    }

    /// The declaration of an item a declared block names as its own (a
    /// crop's seeds): a copy of the template block's item with this item's
    /// differences.
    pub fn block_item(&self, id: &str, template: &str) -> Result<(Value, Vec<String>), String> {
        let item = self.items.get(id).ok_or("not in the oracle")?;
        let block = self
            .blocks
            .get(template)
            .ok_or("template not in the oracle")?;
        if block.item == "minecraft:air" {
            return Err(format!("template {template} has no item"));
        }
        Ok(self.declare(id, item, &block.item))
    }

    fn declare(&self, id: &str, item: &Item, template: &str) -> (Value, Vec<String>) {
        let mut differences = Vec::new();
        if !item.class.starts_with(VANILLA_PACKAGE) {
            differences.push(format!("class {} (behaviour beyond Item)", item.class));
        }
        let base = self.items.get(template);
        let mut decl = json!({"template": template});
        if item.max_stack_size != base.map_or(64, |b| b.max_stack_size) {
            decl["max_stack_size"] = json!(item.max_stack_size);
        }
        if item.rarity != base.map_or("common", |b| b.rarity.as_str()) {
            decl["rarity"] = json!(item.rarity);
        }
        // Ids the content mod may name: vanilla's and its own.
        let own = id.split_once(':').map_or("", |(ns, _)| ns);
        let nameable = |r: &str| {
            r.split_once(':')
                .is_some_and(|(ns, _)| ns == "minecraft" || ns == own)
        };
        let mut components = serde_json::Map::new();
        match &item.food {
            Some(food) => food_components(food, item, &nameable, &mut components, &mut differences),
            None if base.is_some_and(|b| b.food.is_some()) => {
                components.insert("!minecraft:food".into(), json!({}));
                components.insert("!minecraft:consumable".into(), json!({}));
            }
            None => {}
        }
        if !components.is_empty() {
            decl["components"] = Value::Object(components);
        }
        if let Some(r) = &item.crafting_remainder {
            if nameable(r) {
                decl["crafting_remainder"] = json!(r);
            } else {
                differences.push(format!("crafting remainder {r} (another mod's item)"));
            }
        }
        (decl, differences)
    }

    /// A tag's members the content mod may name: vanilla's and its own.
    pub fn tag_members(&self, kind: &str, tag: &str, own: &str) -> Option<Vec<String>> {
        let map = if kind == "block" {
            &self.tags.block
        } else {
            &self.tags.item
        };
        let members = map.get(tag)?;
        Some(
            members
                .iter()
                .filter(|m| {
                    m.split_once(':')
                        .is_some_and(|(ns, _)| ns == "minecraft" || ns == own)
                })
                .cloned()
                .collect(),
        )
    }
}

/// 1.21.1's food as 26.3's `food`, `consumable` and `use_remainder`
/// components. Effects other mods add are left out, as differences.
fn food_components(
    food: &Value,
    item: &Item,
    nameable: &dyn Fn(&str) -> bool,
    out: &mut serde_json::Map<String, Value>,
    differences: &mut Vec<String>,
) {
    let mut f = json!({
        "nutrition": food["nutrition"].as_u64().unwrap_or(0),
        "saturation": food["saturation"].as_f64().unwrap_or(0.0),
    });
    if food["can_always_eat"].as_bool() == Some(true) {
        f["can_always_eat"] = json!(true);
    }
    out.insert("minecraft:food".into(), f);
    let drink = item.use_animation.as_deref() == Some("drink");
    let mut effects = Vec::new();
    for e in food["effects"].as_array().into_iter().flatten() {
        let id = e["id"].as_str().unwrap_or("");
        if !id.starts_with("minecraft:") {
            differences.push(format!("effect {id} when eaten (no such effect)"));
            continue;
        }
        effects.push(json!({
            "type": "minecraft:apply_effects",
            "effects": [{
                "id": id,
                "duration": e["duration"].as_i64().unwrap_or(0),
                "amplifier": e["amplifier"].as_i64().unwrap_or(0),
            }],
            "probability": e["probability"].as_f64().unwrap_or(1.0),
        }));
    }
    out.insert(
        "minecraft:consumable".into(),
        json!({
            "consume_seconds": food["eat_seconds"].as_f64().unwrap_or(1.6),
            "animation": if drink { "drink" } else { "eat" },
            "sound": if drink { "minecraft:entity.generic.drink" } else { "minecraft:entity.generic.eat" },
            "has_consume_particles": !drink,
            "on_consume_effects": effects,
        }),
    );
    match food["using_converts_to"].as_str() {
        Some(r) if nameable(r) => {
            out.insert(
                "minecraft:use_remainder".into(),
                json!({"id": r, "count": 1}),
            );
        }
        Some(r) => differences.push(format!("leaves {r} when eaten (another mod's item)")),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(hardness: f32, sound: &str, outline: Value) -> Value {
        json!({"hardness": hardness, "emission": 0, "requires_tool": false, "map_color_id": 13,
            "sound": {"break": sound}, "outline": outline, "collision": outline})
    }
    fn block(class: &[&str], props: Value, states: Value) -> Value {
        let default = states
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .to_owned();
        json!({"class": class, "properties": props, "default": default, "item": "x",
            "explosion_resistance": 3.0, "friction": 0.6, "speed_factor": 1.0,
            "jump_factor": 1.0, "states": states})
    }

    #[test]
    fn templates_follow_class_shapes_and_sounds() {
        let cube = json!([[0, 0, 0, 1, 1, 1]]);
        let low = json!([[0, 0, 0, 1, 0.9375, 1]]);
        let block_class = ["net.minecraft.world.level.block.Block"];
        let path = ["net.minecraft.world.level.block.DirtPathBlock"];
        let facts: Facts = serde_json::from_value(json!({
            "blocks": {
                "minecraft:stone": block(&block_class, json!({}), json!({"": state(1.5, "stone", cube.clone())})),
                "minecraft:oak_planks": block(&block_class, json!({}), json!({"": state(2.0, "wood", cube.clone())})),
                "minecraft:dirt_path": block(&path, json!({}), json!({"": state(0.65, "grass", low.clone())})),
                "m:wood_tile": block(&["m.Tile", block_class[0]], json!({}), json!({"": state(2.5, "wood", cube.clone())})),
                "m:path": block(&["m.Path", block_class[0]], json!({}), json!({"": state(1.0, "stone", low.clone())})),
                "m:gadget": block(&["m.Gadget", block_class[0]], json!({"mode": ["a", "b"]}),
                    json!({"mode=a": state(1.0, "stone", cube.clone()), "mode=b": state(1.0, "stone", cube)})),
            },
            "items": {}
        }))
        .unwrap();
        let any = |_: &str| true;
        let tile = facts.template("m:wood_tile", &any).unwrap();
        assert_eq!(tile.template, "minecraft:oak_planks");
        assert_eq!(tile.properties["hardness"], json!(2.5));
        assert!(!tile.properties.contains_key("sounds"));
        assert_eq!(
            tile.differences,
            ["class m.Tile (behaviour beyond net.minecraft.world.level.block.Block)"]
        );
        // A plain Block is never templated on a DirtPathBlock: shapes differ, and say so.
        let path = facts.template("m:path", &any).unwrap();
        assert_eq!(path.template, "minecraft:stone");
        assert!(
            path.differences[0].starts_with("shapes differ"),
            "{:?}",
            path.differences
        );
        assert_eq!(facts.template("m:gadget", &any), None);
        let tile = facts
            .template("m:wood_tile", &|n: &str| n != "minecraft:oak_planks")
            .unwrap();
        assert_eq!(tile.template, "minecraft:stone");
        assert_eq!(tile.differences[0], "sounds: no vanilla block has them");
    }

    #[test]
    fn drops_itself_is_vanillas_drop_self() {
        let cube = json!([[0, 0, 0, 1, 1, 1]]);
        let with = |requires_tool: bool, drops: Value| {
            let mut s = state(1.5, "stone", cube.clone());
            s["requires_tool"] = json!(requires_tool);
            s["drops"] = drops;
            let b: Block = serde_json::from_value(block(&[], json!({}), json!({"": s}))).unwrap();
            b.drops_itself("m:tile")
        };
        let me = json!({"m:tile": 1});
        let always = Some(SelfDrop::Always);
        assert_eq!(with(false, json!({"hand": me, "shears": me})), always);
        // A state that needs a tool drops nothing by hand, as dropSelf does.
        assert_eq!(
            with(true, json!({"hand": {}, "diamond_pickaxe": me})),
            always
        );
        assert_eq!(
            with(false, json!({"hand": {}, "diamond_pickaxe": me})),
            None
        );
        // Two of itself (a double slab), or something else, is not dropSelf.
        assert_eq!(with(false, json!({"hand": {"m:tile": 2}})), None);
        assert_eq!(with(false, json!({"hand": {"m:other": 1}})), None);
        assert_eq!(with(false, json!({})), None);
    }

    #[test]
    fn a_door_drops_from_its_lower_half() {
        let cube = json!([[0, 0, 0, 1, 1, 1]]);
        let me = json!({"hand": {"m:door": 1}, "shears": {"m:door": 1}});
        let none = json!({"hand": {}, "shears": {}});
        let door = |upper: &Value| {
            let mut states = serde_json::Map::new();
            for (key, drops) in [
                ("half=lower,open=false", &me),
                ("half=lower,open=true", &me),
                ("half=upper,open=false", upper),
                ("half=upper,open=true", upper),
            ] {
                let mut s = state(3.0, "wood", cube.clone());
                s["drops"] = drops.clone();
                states.insert(key.to_owned(), s);
            }
            let props = json!({"half": ["upper", "lower"], "open": ["true", "false"]});
            let b: Block =
                serde_json::from_value(block(&[], props, Value::Object(states))).unwrap();
            b.drops_itself("m:door")
        };
        assert_eq!(
            door(&none),
            Some(SelfDrop::When("half".to_owned(), "lower".to_owned()))
        );
        assert_eq!(door(&me), Some(SelfDrop::Always));
        // Something else from the upper half: not a door table.
        assert_eq!(door(&json!({"hand": {"m:other": 1}})), None);
    }

    #[test]
    fn foods_and_seeds_declare_their_components() {
        let crop = ["net.minecraft.world.level.block.CropBlock"];
        let mut wheat = block(&crop, json!({}), json!({"": state(0.0, "crop", json!([]))}));
        wheat["item"] = json!("minecraft:wheat_seeds");
        let mut m_crop = wheat.clone();
        m_crop["item"] = json!("m:seeds");
        let mut wall_sign = wheat.clone();
        wall_sign["item"] = json!("m:sign");
        let item = |class: &str, extra: Value| {
            let mut v = json!({"class": class, "max_stack_size": 64, "rarity": "common"});
            v.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            v
        };
        let vanilla = "net.minecraft.world.item.Item";
        let facts: Facts = serde_json::from_value(json!({
            "blocks": {"minecraft:wheat": wheat, "m:crop": m_crop, "m:wall_sign": wall_sign},
            "items": {
                "minecraft:wheat_seeds": item(vanilla, json!({"block": "minecraft:wheat"})),
                "m:stew": item("m.Consumable", json!({
                    "max_stack_size": 16, "use_animation": "eat",
                    "crafting_remainder": "minecraft:bowl",
                    "food": {"nutrition": 10, "saturation": 12.0, "can_always_eat": true,
                        "eat_seconds": 1.6, "using_converts_to": "minecraft:bowl",
                        "effects": [
                            {"id": "minecraft:regeneration", "duration": 100, "amplifier": 0, "probability": 1.0},
                            {"id": "m:comfort", "duration": 600, "amplifier": 0, "probability": 1.0}
                        ]}})),
                "m:cider": item(vanilla, json!({"use_animation": "drink",
                    "crafting_remainder": "other:mug",
                    "food": {"nutrition": 2, "saturation": 0.4, "eat_seconds": 1.6, "effects": []}})),
                "m:seeds": item(vanilla, json!({"block": "m:crop"})),
                "m:sign": item(vanilla, json!({"block": "m:sign"})),
            }
        }))
        .unwrap();

        let (stew, differences) = facts.item("m:stew").unwrap();
        assert_eq!(
            stew,
            json!({
                "template": "minecraft:bread",
                "max_stack_size": 16,
                "crafting_remainder": "minecraft:bowl",
                "components": {
                    "minecraft:food": {"nutrition": 10, "saturation": 12.0, "can_always_eat": true},
                    "minecraft:consumable": {
                        "consume_seconds": 1.6, "animation": "eat",
                        "sound": "minecraft:entity.generic.eat", "has_consume_particles": true,
                        "on_consume_effects": [{"type": "minecraft:apply_effects",
                            "effects": [{"id": "minecraft:regeneration", "duration": 100, "amplifier": 0}],
                            "probability": 1.0}]
                    },
                    "minecraft:use_remainder": {"id": "minecraft:bowl", "count": 1}
                }
            })
        );
        assert_eq!(
            differences,
            [
                "class m.Consumable (behaviour beyond Item)",
                "effect m:comfort when eaten (no such effect)"
            ]
        );
        let (cider, differences) = facts.item("m:cider").unwrap();
        assert_eq!(
            cider["components"]["minecraft:consumable"]["animation"],
            "drink"
        );
        assert!(cider.get("crafting_remainder").is_none());
        assert_eq!(
            differences,
            ["crafting remainder other:mug (another mod's item)"]
        );

        // A crop names its seeds; a wall sign does not name its standing
        // sign's item.
        assert_eq!(
            facts.named_item("m:crop", "minecraft:wheat"),
            Some("m:seeds")
        );
        assert_eq!(facts.named_item("m:wall_sign", "minecraft:wheat"), None);
        // A crop's seeds copy the template crop's item.
        let (seeds, differences) = facts.block_item("m:seeds", "minecraft:wheat").unwrap();
        assert_eq!(seeds, json!({"template": "minecraft:wheat_seeds"}));
        assert!(differences.is_empty());
    }

    #[test]
    fn state_keys_sort() {
        assert_eq!(
            state_key("half=lower,facing=north"),
            "facing=north,half=lower"
        );
        assert_eq!(state_key(""), "");
    }
}
