"""Kiloton's blocks and items for Horizon, as content declarations (data/kiloton/horizon/block|item/*.json).

    python tools/gen_registry.py     write them into horizon-26.3/content/ (then tools/sync_resources.py)

The table mirrors the Java registries (shared-26.3: registry/ModBlocks.java, ModItems.java): the same ids,
strength, sounds, map colours, light, state properties, shapes, stack sizes and rarities. Horizon declares a
block by a vanilla *template* whose behaviour class it takes; Kiloton's blocks get inert templates (iron block,
stone, ores, a carpet, leaves) and their own state properties, because their behaviour (machines, the bomb, the
radar) will be Kiloton's own code, not a furnace's or a bed's.

Why files and not code: a trait mod can't build `horizon_mods::content::Block` values (its package may not depend
on the horizon-mods path crate, and horizon-client doesn't re-export it), so `Hooks::blocks` is out of reach.
"""

from __future__ import annotations

import json
import shutil
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
OUT = HERE / "content" / "data" / "kiloton" / "horizon"

# MapColor ids (net.minecraft.world.level.material.MapColor).
MAP = {"STONE": 11, "METAL": 6, "SNOW": 8, "COLOR_YELLOW": 18, "COLOR_LIGHT_GREEN": 19, "COLOR_GRAY": 21,
       "COLOR_LIGHT_GRAY": 22, "COLOR_PURPLE": 24, "COLOR_BLUE": 25, "COLOR_GREEN": 27, "TERRACOTTA_ORANGE": 37,
       "DEEPSLATE": 59, "COLOR_ORANGE": 15, "GOLD": 30, "DIAMOND": 31}
# SoundType -> a vanilla block with those sounds.
SOUND = {"METAL": "minecraft:iron_block", "GLASS": "minecraft:glass", "SAND": "minecraft:sand",
         "GRASS": "minecraft:grass_block", "DEEPSLATE": "minecraft:deepslate", "STONE": "minecraft:stone",
         "COPPER": "minecraft:copper_block"}

HORIZONTAL = {"type": "enum", "values": ["north", "south", "west", "east"]}  # HorizontalDirectionalBlock.FACING
ALL_FACING = {"type": "enum", "values": ["down", "up", "north", "south", "west", "east"]}  # BlockStateProperties.FACING
BOOL = {"type": "boolean"}
CABLE_SIDE = {"type": "enum", "values": ["none", "cable", "block", "extract"]}  # energy.CableSide


def ints(lo: int, hi: int) -> dict:
    """An IntegerProperty, as an enum of its values in order."""
    return {"type": "enum", "values": [str(i) for i in range(lo, hi + 1)]}


def box(x1, y1, z1, x2, y2, z2) -> list[float]:
    """Block.box: pixels to block units."""
    return [round(v / 16.0, 6) for v in (x1, y1, z1, x2, y2, z2)]


def shapes_by(prop: str, table: dict, default=None) -> dict:
    out = {"property": prop, "values": table}
    if default is not None:
        out["default"] = default
    return out


def block(id, template, *, hardness, resistance=None, sound="STONE", map=None, light=None, tool=False,
          occludes=None, props=None, default=None, shapes=None, state_light=None, item=True, components=None):
    props_out = {"hardness": hardness, "explosion_resistance": resistance if resistance is not None else hardness,
                 "sounds": SOUND[sound]}
    if map is not None:
        props_out["map_color"] = MAP[map]
    if light is not None:
        props_out["light"] = light
    if tool:
        props_out["requires_tool"] = True
    if occludes is not None:
        props_out["occludes"] = occludes
    d = {"template": template, "properties": props_out}
    if props:
        d["state_properties"] = props
    if default:
        d["default_state"] = default
    if shapes:
        d["shapes"] = shapes
    if state_light:
        d["state_light"] = state_light
    d["item"] = item
    if components:
        d["components"] = components
    return id, d


def lit_light(level: int) -> dict:
    return {"property": "lit", "values": {"false": 0, "true": level}}


BOMB_SHAPES = shapes_by("facing", {"north": [box(0, 0, 1.5, 16, 15, 14.5)], "south": [box(0, 0, 1.5, 16, 15, 14.5)],
                                   "east": [box(1.5, 0, 0, 14.5, 15, 16)], "west": [box(1.5, 0, 0, 14.5, 15, 16)]})
EPIC_SINGLE = {"minecraft:max_stack_size": 1, "minecraft:rarity": "epic"}
BOMB_TEMPLATE = "minecraft:crafting_table"
# The device's menu (Java: BombMenu, the same for both casings): slots 0..15 as NukeBlockEntity's (pit 0..5, tamper 6,
# fuel 7..12, spark plug 13, jacket 14), then 15 the panel never shows, where an armed device keeps its load so that
# nobody takes a core out of it (Java: removeItem refuses while armed). Data slots: api/src/bomb.rs, `DATA`.
BOMB_MENU = {"slots": 30, "data_slots": 12}
# A machine's menu (Java: MachineMenu): its five slots (input A, input B, fuel, output A, output B; the screen shows the
# ones its kind uses) and 13 data slots (api/src/machines.rs, `data`).
MACHINE_MENU = {"slots": 5, "data_slots": 13}
MACHINES = ["uranium_mill", "exchange_column", "gas_centrifuge", "breeder_reactor", "core_foundry"]
MENUS = [("nuclear_bomb", BOMB_MENU), ("thermonuclear_bomb", BOMB_MENU)] + [(m, MACHINE_MENU) for m in MACHINES]
SINGLE = {"minecraft:max_stack_size": 1}


def machine(id, light=0, centrifuge=False):
    # The crafting table's class, as the bomb's: a mod block's menu opens only when its template opens one.
    return block(id, BOMB_TEMPLATE, hardness=3.5, resistance=6.0, sound="METAL", map="METAL", tool=True,
                 occludes=False, props={"facing": HORIZONTAL, "lit": BOOL}, default={"facing": "north", "lit": "false"},
                 state_light=lit_light(light) if light else None,
                 shapes={"default": [box(0, 0, 0, 16, 2, 16), box(3, 2, 3, 13, 16, 13)]} if centrifuge else None)


CABLE_SIDES = ("down", "up", "north", "south", "west", "east")


def cable_turn(d: str, x: float, y: float, z: float) -> tuple:
    """CableBlock.turn: a point of a box modelled pointing north, turned to face d."""
    return {"north": (x, y, z), "south": (16 - x, y, 16 - z), "east": (16 - z, y, x), "west": (z, y, 16 - x),
            "up": (x, 16 - z, y), "down": (x, z, 16 - y)}[d]


def cable_box(d: str, x0, y0, z0, x1, y1, z1) -> list[float]:
    a, b = cable_turn(d, x0, y0, z0), cable_turn(d, x1, y1, z1)
    return box(min(a[0], b[0]), min(a[1], b[1]), min(a[2], b[2]), max(a[0], b[0]), max(a[1], b[1]), max(a[2], b[2]))


def cable_shapes() -> dict:
    """CableBlock.getShape for every state: the core, an arm on each side that isn't none, a plate where it touches a
    block. Horizon keys a table by every property's value, so all 4096 states are written (compact: see main)."""
    import itertools
    core = box(5, 5, 5, 11, 11, 11)
    arms = {d: cable_box(d, 5, 5, 0, 11, 11, 5) for d in CABLE_SIDES}
    plates = {d: cable_box(d, 3, 3, 0, 13, 13, 1) for d in CABLE_SIDES}
    values = {}
    for t in itertools.product(CABLE_SIDE["values"], repeat=6):
        if all(v == "none" for v in t):
            continue
        boxes = [core]
        for d, v in zip(CABLE_SIDES, t):
            if v != "none":
                boxes.append(arms[d])
            if v in ("block", "extract"):
                boxes.append(plates[d])
        values[",".join(t)] = boxes
    return {"properties": list(CABLE_SIDES), "values": values, "default": [core]}


BLOCKS = [
    # Two blocks long (head + foot, like a bed). The template is the crafting table only because a mod block's
    # menu opens when its template's class opens one (horizon-world level/interact.rs); the crafting table has no
    # state properties, burns not, and no villager works at it. The panel is Kiloton's own (api/src/bomb.rs).
    block("nuclear_bomb", BOMB_TEMPLATE, hardness=5.0, resistance=12.0, sound="METAL", map="COLOR_YELLOW",
          tool=True, occludes=False, props={"facing": HORIZONTAL, "part": {"type": "enum", "values": ["head", "foot"]},
                                            "armed": BOOL},
          default={"facing": "north", "part": "head", "armed": "false"}, shapes=BOMB_SHAPES, components=EPIC_SINGLE),
    block("thermonuclear_bomb", BOMB_TEMPLATE, hardness=5.0, resistance=12.0, sound="METAL",
          map="COLOR_LIGHT_GRAY", tool=True, occludes=False,
          props={"facing": HORIZONTAL, "part": {"type": "enum", "values": ["head", "foot"]}, "armed": BOOL},
          default={"facing": "north", "part": "head", "armed": "false"}, shapes=BOMB_SHAPES, components=EPIC_SINGLE),
    block("trinitite", "minecraft:stone", hardness=1.2, resistance=4.0, sound="GLASS", map="COLOR_LIGHT_GREEN", tool=True),
    # A film of fallout a pixel thick, like a carpet; soot=true where the black rain fell.
    block("fallout_dust", "minecraft:white_carpet", hardness=0.1, sound="SAND", map="COLOR_LIGHT_GRAY",
          props={"soot": BOOL}, default={"soot": "false"}),
    # Foliage killed by the radiation: leaves that never decay.
    block("irradiated_leaves", "minecraft:oak_leaves", hardness=0.2, sound="GRASS", map="TERRACOTTA_ORANGE",
          default={"persistent": "true"}),
    block("uranium_ore", "minecraft:iron_ore", hardness=3.0, resistance=3.0, map="STONE", tool=True),
    block("deepslate_uranium_ore", "minecraft:deepslate_iron_ore", hardness=4.5, resistance=3.0, sound="DEEPSLATE",
          map="DEEPSLATE", tool=True),
    block("lithium_ore", "minecraft:iron_ore", hardness=3.0, resistance=3.0, map="STONE", tool=True),
    block("deepslate_lithium_ore", "minecraft:deepslate_iron_ore", hardness=4.5, resistance=3.0, sound="DEEPSLATE",
          map="DEEPSLATE", tool=True),
    machine("uranium_mill"),
    machine("exchange_column"),
    machine("gas_centrifuge", centrifuge=True),
    machine("breeder_reactor", light=10),
    machine("core_foundry", light=13),
    block("solar_panel", "minecraft:iron_block", hardness=1.5, resistance=3.0, sound="METAL", map="COLOR_BLUE",
          occludes=False, shapes={"default": [box(0, 0, 0, 16, 5, 16)]}),
    block("creative_source", "minecraft:iron_block", hardness=1.5, resistance=3.0, sound="METAL", map="COLOR_PURPLE",
          light=9),
    block("rtg", "minecraft:iron_block", hardness=3.5, resistance=6.0, sound="METAL", map="METAL", tool=True,
          occludes=False, props={"lit": BOOL}, default={"lit": "false"}, state_light=lit_light(6),
          shapes={"default": [box(1.5, 0, 1.5, 14.5, 16, 14.5)]}),
    block("air_filter", "minecraft:iron_block", hardness=3.5, resistance=6.0, sound="METAL", map="METAL", tool=True,
          occludes=False, props={"facing": ALL_FACING, "lit": BOOL}, default={"facing": "north", "lit": "false"}),
    block("radar", "minecraft:iron_block", hardness=3.5, resistance=6.0, sound="METAL", map="METAL", tool=True,
          occludes=False, props={"powered": BOOL}, default={"powered": "false"}),
    # The radar's 3x3 structure around its controller; part 9 is the mast.
    block("radar_part", "minecraft:iron_block", hardness=3.5, resistance=6.0, sound="METAL", map="METAL", tool=True,
          occludes=False, props={"part": ints(0, 9)}, default={"part": "0"},
          shapes=shapes_by("part", {"9": [box(3, 0, 3, 13, 16, 13)]}, default=[box(0, 0, 0, 16, 16, 16)]), item=False),
    block("radar_display", "minecraft:iron_block", hardness=3.0, resistance=6.0, sound="METAL", map="METAL", tool=True,
          props={"facing": HORIZONTAL, "up": BOOL, "down": BOOL, "left": BOOL, "right": BOOL},
          default={"facing": "north", "up": "false", "down": "false", "left": "false", "right": "false"}),
    block("launch_pad", "minecraft:iron_block", hardness=5.0, resistance=12.0, sound="METAL", map="METAL", tool=True,
          occludes=False),
    # The missiles standing on their pads: three blocks, unbreakable; their items are separate (below).
    block("missile", "minecraft:iron_block", hardness=-1.0, resistance=3600000.0, sound="METAL", map="SNOW",
          occludes=False, props={"part": ints(1, 3)}, default={"part": "1"},
          shapes=shapes_by("part", {"1": [box(0.5, 0, 0.5, 15.5, 16, 15.5)]}, default=[box(4.25, 0, 4.25, 11.75, 16, 11.75)]),
          item=False),
    block("icbm", "minecraft:iron_block", hardness=-1.0, resistance=3600000.0, sound="METAL", map="SNOW",
          occludes=False, props={"part": ints(1, 3)}, default={"part": "1"},
          shapes=shapes_by("part", {"1": [box(0.5, 0, 0.5, 15.5, 16, 15.5)]}, default=[box(4.25, 0, 4.25, 11.75, 16, 11.75)]),
          item=False),
    block("launch_console", "minecraft:iron_block", hardness=3.5, resistance=6.0, sound="METAL", map="METAL", tool=True,
          occludes=False, props={"facing": HORIZONTAL}, default={"facing": "north"},
          shapes={"default": [box(0, 0, 0, 16, 12, 16)]}, components=SINGLE),
    # The plate sits on the wall behind it (facing south, the wall is to the north).
    block("launch_key", "minecraft:iron_block", hardness=2.0, resistance=6.0, sound="METAL", map="METAL",
          occludes=False, props={"facing": HORIZONTAL, "turned": BOOL}, default={"facing": "north", "turned": "false"},
          shapes=shapes_by("facing", {"south": [box(3, 3, 0, 13, 13, 4)], "west": [box(12, 3, 3, 16, 13, 13)],
                                      "north": [box(3, 3, 12, 13, 13, 16)], "east": [box(0, 3, 3, 4, 13, 13)]})),
    block("air_raid_siren", "minecraft:iron_block", hardness=3.5, resistance=6.0, sound="METAL", map="COLOR_GRAY",
          tool=True, occludes=False, props={"on": BOOL}, default={"on": "false"},
          shapes={"default": [box(2, 0, 2, 14, 15, 14)]}),
    block("sam_launcher", "minecraft:iron_block", hardness=5.0, resistance=12.0, sound="METAL", map="COLOR_GREEN",
          tool=True, occludes=False, shapes={"default": [box(0, 0, 0, 16, 8, 16), box(4, 8, 4, 12, 16, 12)]},
          components=SINGLE),
    # The interceptor battery's deck around its controller; parts 0 and 8 carry a cabinet.
    block("sam_part", "minecraft:iron_block", hardness=5.0, resistance=12.0, sound="METAL", map="COLOR_GREEN",
          tool=True, occludes=False, props={"part": ints(0, 10)}, default={"part": "0"},
          shapes=shapes_by("part", {"0": [box(0, 0, 0, 16, 8, 16), box(3, 8, 3, 13, 12, 13)],
                                    "8": [box(0, 0, 0, 16, 8, 16), box(3, 8, 3, 13, 12, 13)]},
                           default=[box(0, 0, 0, 16, 8, 16)]), item=False),
    block("burst_detector", "minecraft:iron_block", hardness=3.5, resistance=6.0, sound="METAL", map="METAL", tool=True,
          props={"powered": BOOL}, default={"powered": "false"}),
    block("decon_shower", "minecraft:iron_block", hardness=1.5, resistance=3.0, sound="METAL", map="METAL",
          occludes=False, props={"on": BOOL, "powered": BOOL}, default={"on": "false", "powered": "false"},
          shapes={"default": [box(4, 9, 4, 12, 16, 12)]}),
    block("warhead_debris", "minecraft:iron_block", hardness=1.0, resistance=3.0, sound="METAL", map="COLOR_GRAY",
          occludes=False, shapes={"default": [box(1, 0, 1, 15, 4, 15)]}),
    # 1.4.0: clear green glass with bubbles, between the trinitite and plain glass on sandy ground.
    block("atomic_glass", "minecraft:glass", hardness=0.3, sound="GLASS", map="COLOR_LIGHT_GREEN", occludes=False),
] + [
    # 1.4.0: the energy cables; each side shows nothing, a run to the next cable, a plate on a block, or the amber
    # extraction plate. The shape follows the six sides as CableBlock.getShape (cable_shapes above).
    block(name, "minecraft:iron_block", hardness=0.8, resistance=3.0, sound="COPPER", map=colour, occludes=False,
          props={d: CABLE_SIDE for d in ("down", "up", "north", "south", "west", "east")},
          default={d: "none" for d in ("down", "up", "north", "south", "west", "east")},
          shapes=cable_shapes())
    for name, colour in (("copper_cable", "COLOR_ORANGE"), ("gold_cable", "GOLD"), ("hardened_cable", "DIAMOND"),
                         ("superconducting_cable", "COLOR_PURPLE"))
]

PLAIN = "minecraft:brick"  # an Item with nothing but the default components


def item(id, template=PLAIN, *, stack=None, rarity=None, components=None):
    d = {"template": template}
    if stack is not None:
        d["max_stack_size"] = stack
    if rarity is not None:
        d["rarity"] = rarity
    if components:
        d["components"] = components
    return id, d


def material(id, stack, rarity):
    return item(id, stack=stack, rarity=rarity)


# ArmorType durability multipliers (helmet 11, chestplate 16, leggings 15, boots 13) times the material's.
PIECES = {"helmet": ("head", 11), "chestplate": ("chest", 16), "leggings": ("legs", 15), "boots": ("feet", 13)}


def armor(id, piece, *, durability, defense, toughness, knockback, asset, sound, enchantable, repair, rarity=None,
          extra=None):
    slot, mult = PIECES[piece]
    mods = [{"type": "minecraft:armor", "amount": float(defense), "id": f"minecraft:armor.{piece}",
             "operation": "add_value", "slot": slot},
            {"type": "minecraft:armor_toughness", "amount": float(toughness), "id": f"minecraft:armor.{piece}",
             "operation": "add_value", "slot": slot}]
    if knockback:
        mods.append({"type": "minecraft:knockback_resistance", "amount": knockback, "id": f"minecraft:armor.{piece}",
                     "operation": "add_value", "slot": slot})
    comps = {"minecraft:attribute_modifiers": mods,
             "minecraft:equippable": {"asset_id": asset, "equip_sound": sound, "slot": slot},
             "minecraft:enchantable": {"value": enchantable}, "minecraft:repairable": {"items": repair}}
    if durability:
        comps["minecraft:max_damage"] = durability * mult
    else:
        # Unbreakable: no durability at all (no vanilla item has minecraft:unbreakable, so Horizon refuses it).
        comps["!minecraft:max_damage"] = {}
        comps["!minecraft:damage"] = {}
    if extra:
        comps.update(extra)
    return item(id, f"minecraft:iron_{piece}", rarity=rarity, components=comps)


LEATHER = "minecraft:item.armor.equip_leather"
NETHERITE = "minecraft:item.armor.equip_netherite"

ITEMS = [
    item("geiger_counter", stack=1),
    item("detonator", stack=1, rarity="uncommon"),
    item("long_range_detonator", stack=1, rarity="rare"),
    material("raw_uranium", 64, "common"),
    material("yellowcake", 64, "common"),
    material("low_enriched_uranium", 64, "common"),
    material("highly_enriched_uranium", 64, "uncommon"),
    material("weapons_grade_uranium", 64, "rare"),
    material("depleted_uranium", 64, "common"),
    material("plutonium", 64, "rare"),
    material("uranium_core", 16, "epic"),
    material("plutonium_core", 16, "epic"),
    material("uranium_tamper", 16, "uncommon"),
    # Cells.LIFE: its durability is the fuel.
    item("nuclear_cell", stack=1, rarity="uncommon", components={"minecraft:max_damage": 144000, "minecraft:damage": 0}),
    item("shelter_meter", stack=1),
    material("rocket_motor", 16, "common"),
    material("guidance_unit", 16, "uncommon"),
    material("missile", 1, "rare"),
    material("raw_lithium", 64, "common"),
    material("lithium", 64, "common"),
    material("lithium_6", 64, "uncommon"),
    material("heavy_water", 64, "uncommon"),
    material("lithium_deuteride", 16, "epic"),
    material("icbm", 1, "epic"),
    material("interceptor", 4, "uncommon"),
    item("potassium_iodide", stack=16),
    # Hazmat: leather's protection; the mask's durability is its filter (Hazmat.MASK 55, SUIT 12), repaired with coal.
    armor("hazmat_mask", "helmet", durability=55, defense=1, toughness=0, knockback=0, asset="kiloton:hazmat",
          sound=LEATHER, enchantable=10, repair="#minecraft:coals"),
    armor("hazmat_suit", "chestplate", durability=12, defense=2, toughness=0, knockback=0, asset="kiloton:hazmat",
          sound=LEATHER, enchantable=10, repair="#minecraft:coals"),
    armor("hazmat_trousers", "leggings", durability=12, defense=2, toughness=0, knockback=0, asset="kiloton:hazmat",
          sound=LEATHER, enchantable=10, repair="#minecraft:coals"),
    armor("hazmat_boots", "boots", durability=12, defense=1, toughness=0, knockback=0, asset="kiloton:hazmat",
          sound=LEATHER, enchantable=10, repair="#minecraft:coals"),
]
# The exosuit (ExosuitItem.MATERIAL: toughness 3, knockback 0.1, netherite's repair tag), unbreakable and fireproof.
EXO = {"minecraft:damage_resistant": {"types": "#minecraft:is_fire"}}
for piece, defense in (("helmet", 3), ("chestplate", 8), ("leggings", 6), ("boots", 3)):
    ITEMS.append(armor(f"exosuit_{piece}", piece, durability=None, defense=defense, toughness=3.0, knockback=0.1,
                       asset="kiloton:exosuit", sound=NETHERITE, enchantable=12,
                       repair="#minecraft:repairs_netherite_armor", rarity="rare", extra=EXO))


def main() -> None:
    for kind, table in (("block", BLOCKS), ("item", ITEMS), ("menu", MENUS)):
        folder = OUT / kind
        if folder.exists():
            shutil.rmtree(folder)
        folder.mkdir(parents=True)
        ids = [i for i, _ in table]
        assert len(ids) == len(set(ids)), f"duplicate {kind} ids"
        for id, d in table:
            # The cables' shape tables (4096 states) are written compact; the rest as people read them.
            text = json.dumps(d, separators=(",", ":")) if id.endswith("_cable") else json.dumps(d, indent=2)
            (folder / f"{id}.json").write_text(text + "\n", encoding="utf-8", newline="\n")
    print(f"wrote {len(BLOCKS)} blocks, {len(ITEMS)} items and {len(MENUS)} menus into {OUT}")


if __name__ == "__main__":
    main()
