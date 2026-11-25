# RUSTibites Improved

RUSTibites comes in two editions that share one repository:

| | **Faithful** | **Improved** |
|---|---|---|
| Branch | `main` | `improved` |
| Folder | `Z:\RUSTibites` | `Z:\RUSTibites-improved` (a `git worktree` of the same repository) |
| What it is | The game, reproduced as exactly as the port can. Fixes and parity work only. | The same program plus additions. |
| Name | "RUSTibites Faithful" (title card), zips `RUSTibites-alpha-<date>-<hash>.zip` | "RUSTibites Improved", zips `RUSTibites-Improved-alpha-<date>-<hash>.zip` |
| Reference point | tag `faithful-2026-11-17` (the build that was on the Desktop that day) | branched from that tag |

The tag has a backup: `C:\Users\Storme\Documents\RUSTibites-backups\rustibites-faithful-2026-11-17.bundle` (`git bundle`, all refs at that time; restore with `git clone <bundle> <folder>`). The repository has no remote: the disk is the only other copy.

## Rules

1. **Faithful never changes for Improved's sake.** Nothing Improved adds may alter what `main` does. `main` takes bug fixes and parity work only.
2. **Fixes flow one way: `main` into `improved`.** Merge them with `git -C Z:\RUSTibites-improved merge main` (do it after every batch of fixes on `main`, so the two never drift far). Never merge `improved` into `main`. A fix found while working on Improved is made on `main` first (it is a Faithful bug too), then merged across.
3. **A feature that changes how a world behaves is opt-in.** Anything that changes what a tick computes (a number, an order, a new rule) is a `Feature` (`crates/bibites-core/src/improved.rs`) and sits behind its switch, which is OFF unless asked for, so Improved with every switch off is the Faithful simulation. The test that enforces this is the pinned run `evolution_regression_fingerprint` (and `birth_path_is_bit_identical_to_pre_optimization_golden`) in `crates/bibites-core/src/species.rs`: **it must pass unchanged on this branch and its pinned numbers must never be edited here**; a changed number means a feature leaked into the Faithful behaviour. Details of the switches:

   * The switches are `ImprovedFeatures` (a `u64` of bits, `Copy`, all off by default), held by the world as `World::improved`. They are NOT written into a save and NOT part of the state fingerprint: they belong to the run.
   * In the simulation: `if self.improved.is_on(Feature::TheFeature) { the new behaviour } else { exactly what the game does }`, and nothing else may differ with it off, not even the order of random numbers drawn. How to add one is in the module's documentation (a variant, its key / title / description, the tests).
   * Choosing them (the viewer): the environment variable `RUSTIBITES_IMPROVED=key1,key2` (set but empty = every switch off) overrides `improved_features.json` in the state folder (`{"enabled": ["switch_probe"]}`; `./userdata` of the folder it starts from). The start-up log line says what is on and where that came from, and names keys it does not know. The viewer gives the choice to whatever world it holds before every tick it runs (`App::tick_world`). There is no settings-panel entry yet: the first real feature adds its row. The CLI does not read the switches yet.
   * `switch_probe` is a switch that changes nothing; it exists to prove the chain end to end (try `RUSTIBITES_IMPROVED=switch_probe` and read the log line) and can go once there are real features.
4. **Features around the simulation** (menus, tools, editors, overlays, statistics, quality of life) do not need a switch; they may be on by default.
5. **Saves stay in the game's format.** Improved reads and writes the original game's save files. Anything extra goes in extra files or entries beside the game's data, and whether the original game tolerates a particular extra must be tried with the game before it is relied on.
6. **The product's name is in one place.** `crates/bibites-view/src/edition.rs` (`PRODUCT_NAME`, the world window's title). The other two spots that differ from `main` are the default zip name in `tools/release/build_alpha.ps1` and the `--help` header in `crates/bibites-view/src/main.rs`. Keep it that way so merging `main` never conflicts over strings. The icon is the same as Faithful's (`assets/icon/`).

## What Improved adds

Each addition is listed here with what it changes. UI and tool additions need no switch (they change nothing a tick computes); an addition that does
sits behind a simulation switch (rule 3).

### Individually coloured zones

The game draws every zone in the one colour of the "Fertility Color" setting. Here **each zone can have a colour of its own, chosen by the player**: in the Zones
tab of the settings panel, with the **Preview** switched on, a click on a zone of the preview (a zone that is only a dot is hit from a few pixels off) opens a
colour pop-up at the pointer. It has a **colour wheel** (the hue round it, the saturation from the middle out), a bar for the **brightness**, a box showing the
colour from before next to the colour being made, and a **hex field** the colour can be typed into (click it, then type `RRGGBB` or `RGB`, with or without `#`:
six digits take effect at once, Enter takes the colour and closes the pop-up). The zone has the colour as it is made, so the world and the Preview show it at once.
**Done** or a click outside keeps it, **Escape** puts the colour from before back, **Fertility Color** takes the zone's colour away and **All different** gives every
zone of the world a colour of its own spread round the colour wheel (the first zone keeps the Fertility Color). A zone without a colour of its own is drawn in the
Fertility Color as before. The colour is the zone's tint in the world, drawn with the zone's usual opacity (fertility and biomass density) and gradient, so only the
hue changes; the preview shows it too, as does the species panel's Position map.

* The View panel of the bottom-left toolbar has a check box **Individual Zone Colors**: on from the start, it shows the colours the zones were given; off, the
  zones are all in the Fertility Color again while keeping their colours (choosing one with the box off turns it on). It is kept in the user settings
  (`Individually Coloured Zones`).
* Not simulation: no switch, nothing hashed, nothing in a save (saves stay in the game's format): the colours are kept in `zone_colors/<save name>.json` of the
  state folder, **by the zones' names**, under the name of the save the world was saved or loaded as, like the time tags (a save, a quick save and an autosave
  each store them; a load gives the zones their colours back; the files of deleted autosaves are cleaned away; a new world has none). A world that was never
  saved keeps them for the session only. Zones renamed after the colour was chosen keep it in the session, and the file follows the new name at the next save.
* A zone made again under the same name (a zone group's zones when the group changes) keeps its colour.
* Not done: colours in the scenario editor's own preview (its zones belong to a scenario, not a world) and a colour field on the zone cards.
* The code: `zone_colors.rs` (the colours the pop-up offers), `zone_color_ui.rs` (the pop-up), `app/zone_color_actions.rs` (the actions, the check box, the files),
  `zone_preview.rs` (the click on a zone), `improved_toolbar.rs` (the check box's row, added to the toolbar capture), `bibites-core` `ZoneData::tint`, the places
  that draw zones (`render.rs`, `ui_runtime.rs::emit_zone_preview`). Headless: `--zone-color N:R,G,B` (repeatable), `--zone-color-popup N` (with `--hover x,y`).

### Time tags

Manual marks on the world's time: a **Tag time** button in the Stats and Analysis panel (the data graphs page and the gene analysis page) and in the
Ancestral Lineage Graph opens a name field. **Tag now** (or Enter) sets the tag at the world's time now; **a click on the panel's graph sets it at the
moment clicked, so a past event can be tagged too** (Escape cancels, a blank name becomes "Tag N"). A tag is the world's simulated time plus its name,
and it is **one list for every panel**: a tag set in one panel shows in all of them as a coloured line
(a vertical one on the data graphs and on the gene page's time slider, a horizontal one across the lineage tree and mesh at the height of that time on
its axis) with a label chip that has a remove cross. Each panel has its own **Hide tags / Show tags** button; hiding them in one panel leaves the others
as they are (the choice is kept in `time_tag_view.json` of the state folder). A tag older than the span a graph shows is not drawn there.

* Tagging the past: while the name field is open the panel's graph takes the click and a label at the pointer says how long before now that is ("Tag here:
  12m 30s ago"). On the data graphs the click goes to the logged point nearest to the pointer (the one the graph's white hover line stands on); on the gene
  page the time slider's band is the place (a click between two of the log's points is in proportion); on the lineage graph, tree or mesh, the whole view
  is, and the time is the one of the axis under the pointer. A moment before the world began is refused (the label says so) and the field stays open.
* Not simulation: no switch; a tick does not read the tags.
* Not in a save (saves stay in the game's format): the tags are kept in `time_tags/<save name>.json` of the state folder, under the name of the save the
  world was saved or loaded as. A save, a quick save and an autosave each store the tags beside them; loading a save brings its tags back, without the
  tags of a later timeline (set after that save, when the player went back to it). A new world or scenario starts without tags. The files of autosaves
  that were deleted are cleaned away.
* The code: `time_tags.rs` (the model and the maths that put a time on an axis), `time_tags_ui.rs` (buttons, field, chips, lines), `app/time_tag_actions.rs`
  (typing, switches, keeping them with the saves), the drawing hooks in `hud.rs`, `gene_analysis.rs`, `species_panel.rs` and `lineage_graph.rs`, and
  `bibites-io`'s `storage.rs` for the files.

### Music: the jukebox

The game has no music; here the player can have their own. Put music files in the **`Music` folder** (`userdata\Music` of the folder the program starts from; the game
makes it, with a short note in it, when its window opens) and they play one after the other while the game runs, in the title menu and in a world. mp3 is what it is
made for; wav, wma, m4a, aac and flac files play too (Windows' own decoders do it). Files are looked for in the folder and in the folders inside it, and a file put
in while the game is running joins the music within a few seconds.

The settings are on a fourth page of the Settings screens, **Music Settings** (the button under Hotkeys, in the title menu's Settings and in a world's):

* **Play Music**: on from the start; off holds the track where it is, on goes on with it.
* **Shuffle Music**: on from the start: a random order in which every track plays once before any plays again (and the first of a round is never the one that
  just played); off: the order of the file names.
* **Music Volume**: a slider, 50 % from the start. It only changes the music (the loudness is the square of the slider, so the low half is quiet).
* **Now Playing**: the name of the track (its file name) and how many tracks the folder has; it says so when the folder is empty or a file would not play.
* **Open Folder** shows the Music folder in the file manager; **Previous** and **Next** skip a track.

* Not simulation: no switch, nothing hashed, nothing in a save. The three settings are in the user settings (`Play Music`, `Shuffle Music`, `Music Volume`).
* A file the player cannot open is named on the page and skipped (not tried again until it changes). The track's name is its file name (no tags are read).
* Windows only for now: the music goes through the system's own media player (MCI, `winmm.dll`), so the project takes no audio library. On another system the page
  is there and says the music cannot play. The player is `crates/bibites-audio`, the only place of the project with `unsafe` (the viewer keeps `forbid(unsafe_code)`).
* Only the game window plays: screenshots, tests and the command line never make a sound.
* The code: `jukebox.rs` (the folder scan, the order, the engine and its player thread), `crates/bibites-audio` (the player), `improved_settings.rs` (the page and its
  button, made from the Settings captures' own rows and buttons), the page's wiring in `ui_runtime.rs` and `menu.rs`. Headless: `--jukebox-folder DIR` lets a
  screenshot run play DIR (at the volume of the settings), so a picture of the Music page can show a track playing; `--menu --menu-view settings --settings-tab 3`
  opens the Settings screen on the Music page (in a screenshot run or in the window).

### Editable species name list

The game names a species with two words (a genus and an epithet, "Corbinus waldentimotei"), both drawn from one built-in list of words (its supporters' Latin
names). Here **the player can have a list of their own**, a text file: **`species_names.txt`** in `userdata` (the game makes it, with notes in it, when its window
opens). Put words in it, one or several to a line (a line that starts with `#` is a note); a word is the same word whatever its case and a repeated word counts once.
A species is named from the words as the game does it: the genus with a capital letter (a species' descendants inherit it for a few generations, as ever), the
epithet in lower case. The game looks at the file again every second, so a change you save applies to the species named from then on; species that already
have a name keep it, and the saves are as ever (a species' name is its two words, whatever list they came from).

The **Species Names** page of the Settings screens (the fifth button of the column, in the title menu's Settings and in a world's) has:

* **Species Names**: where the names come from. **My List** (the choice from the start): your words, every one as likely as any other; while the file has no words
  the game's list is used. **Game's List**: the file is ignored. **Both Lists**: half of the names from each (the game's half drawn as the game draws it).
* **My List**: how many words the file has and what that means now.
* **Open File** opens the file in the text editor the system has for `.txt` files; **Reload** reads it again at once.
* **Sample Names**: two names as the list in use would give them; **Show New** gives two others.

* Not simulation: no switch, nothing hashed, nothing in a save. Naming draws one random number from the name stream either way, so the run's name stream is
  used the same with a list as without one, and with no list the names are exactly the game's (the pinned runs hold that). `bibites_core::species_names` (`NamePool`) is
  the list; it is process-wide (`set_name_pool`) so every world, whatever thread it ticks on and including the first species of a new world, is named from it.
* A short list runs out of fresh combinations quickly (the generator looks for an unused name a limited number of times, then repeats one), so give it plenty.
* The file is read in as text with the byte-order mark and the usual line endings handled; a file with more than 20,000 words is cut there.
* The code: `crates/bibites-core/src/species_names.rs` (the pool, its draws, the process-wide slot) and its use in `species.rs` (`weighted_random_patron`),
  `name_list.rs` (the file, its watcher, the page's lines), `improved_settings.rs` (the page: a dropdown row, two lines of text and three buttons, made from the
  Settings captures' own rows), the wiring in `ui_runtime.rs`, `menu.rs` and `main.rs`. Headless: `--species-names-file FILE` reads FILE for a screenshot run and
  `--settings-tab 4` opens the page.

### Gene selector tower

The game's colour killer picks bibites by their colour: every cycle it kills a bibite whose hue is OUTSIDE the tolerance around its target hue, so only the target
colour survives. The **gene selector tower** is the same tower for a **gene**. It is the second button from the top of the placer panel (the bottom-left
toolbar's first button; above the radiation tower's, with the DNA icon): click it, click the world to put the tower down (a right click cancels), click the tower to open its panel:

* **Gene**: a list of the 35 genes (a new tower targets Diet); another gene resets the range to that gene's whole range.
* **Follow Zone**: the colour killer's dropdown ("None" and the five zones closest to the tower, each with its distance): the tower goes where its zone goes, at every
  tick, and cannot be dragged away from it; "None" lets it go.
* **Lowest** and **Highest**: the range, two sliders in the gene's own units, kept in order (one dragged past the other takes it along). The **Range** strip shows
  the gene's whole range with the part that is kept in green and the part that is killed in red.
* **Kill: Outside / Inside**: outside (from the start) kills the bibites whose gene is out of the range, so only the range survives, as the colour killer does;
  inside kills the ones in it (a cull).
* **Kills**: how many bibites a cycle kills, 1 to 50 (1 from the start). **Pick: Any / Most**: the first ones found, or the furthest off target (outside: the
  furthest out of the range; inside: the deepest in it).
* **Radius** and **Period**: 400 and 100 s from the start, as the colour killer's. The progress bar, the count of kills and Remove are the colour killer's too.

A new tower has the gene's whole range, so it kills nobody until the range is narrowed. The rules are the colour killer's: every Period seconds the bibites that
touch the radius are looked at (the first 128, in id order), and what the cycle kills dies as the colour killer's victims do.

* A simulation change, so a switch: `Feature::GeneSelectorTower` (`gene_selector_tower`). Settings > **Towers & Streams** has the switch **Gene Selector Towers**: on from
  the start (with no tower in the world nothing changes either way), off = the towers do nothing and are drawn faint (their panel says so). The world gets it before every tick like
  the other switches. A cycle draws no random number, so a world without a gene selector is untouched (the pinned runs hold that).
* Not in a save (the original game has no such tower): the towers are kept in `gene_towers/<save name>.json` of the state folder, by the genes' names, under the
  name of the save the world was saved or loaded as (the zone it follows by the zone's name; a zone that is gone leaves the tower where it was saved), like the time tags and the zone colours (a save, a quick save and an autosave each store them; a load brings
  them back and takes away the earlier world's). The game-format save writer leaves them out, so the original game and the plain edition open the save without them.
* Not done: more than one gene per tower.
* The code: `crates/bibites-core/src/gene_selector.rs` (the tower and its rules), its `Tool::Gene` variant in `tools.rs`, `World::step_gene_selectors` (after the colour
  killers), `tower_panel.rs` (the panel), `improved_toolbar.rs` (the placer button), `app/gene_tower_actions.rs` (the files), `improved_settings.rs` (the Towers
  page), `render.rs` (the tower drawn in the colour of its gene).

### Hydrothermal vent tower

A plant pellet spawner you place in the world. It is the top button of the placer panel (the bottom-left toolbar's first button; the plant pellet icon, above the gene
selector's): click it, click the world to put the vent down (a right click cancels), click the vent to open its panel:

* **Follow Zone**: the colour killer's dropdown ("None" and the five closest zones): the vent goes where its zone goes.
* **Velocity**: 10 to 400 u/s (250 from the start), how fast a pellet leaves the vent, in a random direction. 400 is the world's top speed for anything that moves
  (`max_translation_speed`), so more would only be cut back.
* **Range**: 10 to 3000 u (200 from the start), how far a pellet slides in open ground. The game's drag alone stops a pellet within a few tens of units (about 20 u at
  250 u/s for a pellet of the default size), so the vent lowers the drag of ITS pellet while it flies, to `speed / range` a second (a body that slows exponentially covers
  its speed over its drag), and gives it the game's drag back once it has slowed to 10 u/s. Other pellets, and the vent's own once it has stopped, are the game's
  ordinary pellets: a bibite that pushes one moves it as it would any. A range shorter than the game's own reach changes nothing. Pellets that bump into things (bibites,
  other pellets) stop short of it; the speed decides how soon they get there, not how far. The factor is a field of the pellet store (`drag_factor`, 1 for every pellet
  that is not a flying vent pellet), so a world without a vent is untouched.
* **Pellet Size**: 10 to 5000 E, the energy of a pellet (the world's default pellet size, 200 E, when the vent is placed). **Shoot Rate**: 0.1 to 50 pellets a second
  (2 from the start). Each shot varies by up to a quarter in speed and in size (the pellet placer's variation).
* **Pellet Cap** and **Recycle**: the cap (100 E to 1,000,000 E, 20,000 from the start) counts the energy of the pellets this vent shot that are still in the world;
  the **Filled** bar shows how much of it they fill. With Recycle OFF (from the start) the vent stops shooting at the cap and goes on when bibites have eaten some. With
  it ON the vent goes on shooting and takes the energy of its OLDEST pellets back until it is at the cap (they shrink, then vanish), as a zone does over its Biomass cap
  (`Zone.RecyclePellets`).
* The pellets are ordinary plant pellets: they decay, merge, push and are eaten as any. What a vent makes is booked as produced energy, what it takes back as recycled.

* A simulation change, so a switch: `Feature::HydrothermalVent` (`hydrothermal_vent`). Settings > **Towers & Streams** has the switch **Hydrothermal Vents**: on from the start
  (with no vent in the world nothing changes either way), off = the vents do nothing and are drawn faint (their panel says so). A shot draws three numbers from the
  world's random numbers (direction, speed, size), so a world without a vent is untouched (the pinned runs hold that) and one with a vent runs the same way twice.
* Not in a save: the vents are kept in `vent_towers/<save name>.json` of the state folder the way the gene towers are (their settings, position, the zone by its name,
  the range and the count of pellets shot). The pellets a vent shot ARE in the save, as plain pellets, but a saved world does not remember which vent shot which: after a load the
  vent counts and recycles only what it shoots from then on (the earlier ones stay as ordinary pellets; one that was flying at the save has the game's drag after the load).
* Not done: shooting meat or other materials, a direction or an arc, more than one vent's pellets sharing a cap.
* The code: `crates/bibites-core/src/vent.rs` (the tower and its shot), its `Tool::Vent` variant and the per-vent list of pellets in `tools.rs`, `World::step_vents` and
  `recycle_vent_pellets` (after the pellet producers), checkpoint tag 4, `tower_panel.rs` (the panel), `improved_toolbar.rs` (the placer button), `app/vent_tower_actions.rs`
  (the files), `improved_settings.rs` (the Towers page), `render.rs` (the vent drawn).

### Species Distribution window: zone names and biomass off

The Species Distribution window (the **Position** tab of a species' panel: the time slider over the species' history and the map of the zones with the ellipse where the
species was) writes three lines over the middle of each zone: its name, its biomass (`125.66 kE`) and the pellets that come to (`~26 pellets`). Two check boxes in the map's
lower margin switch them off:

* **Zone names**: the name line. **Zone biomass**: the biomass line and the pellet count under it (the count is the biomass in pellets, so it goes with it). Both are on from
  the start, as the game has it. The lines that stay keep their own places.
* A way of looking, not a setting of the world: nothing in the simulation notices it. The choice is kept in the user settings (`Species Map Zone Names`, `Species Map Zone
  Biomass`: the check boxes are on the window itself, not on the Settings screens), so it is there again at the next start.
* The check boxes are over the map's own cover too, so the choice can be made while the species was not around.
* Not done: the same two switches for the Zones tab's preview of the world's parameters (it carries the same labels; it is left as the game has it), and the zone lines under
  the map ("Possible Zone Presence") which name the zones the species overlapped and are not changed.
* The code: `species_panel/position.rs` (`emit_zone_labels` takes which lines to write; `map_options` draws the check boxes), `app/map_zone_labels.rs` (the actions),
  `user_settings.rs` (the two settings), `main.rs` (applied at start, stored when changed; `--hide-map-zone-names` / `--hide-map-zone-biomass` for pictures).

### Force tower

A tower that PULLS the things near it towards itself, or PUSHES them away: planet-like heaps of pellets, bibites that stay in a zone. It is the top button of the placer
panel (the bottom-left toolbar's first button; a light blue round icon, above the vent's): click it, click the world to put the tower down (a right click cancels), click
the tower to open its panel:

* **Follow Zone**: the colour killer's dropdown ("None" and the five closest zones): the tower goes where its zone goes.
* **Range**: 10 to 5000 u (300 from the start), how far from the tower it acts. While the tower is selected the range is shown as a faint circle (blue for a pull,
  orange for a push).
* **Strength**: -500 to +500 (100 from the start), in u/s per second: positive pulls, negative pushes, 0 does nothing. It is an ACCELERATION, like gravity, not a
  force: a heavy body and a light one gain the same speed from it, so the tower does not favour the small. Every tick it adds `strength * falloff(distance) * dt` to the
  velocity of each body in range (along the line to the tower, or away from it); the world's own drag and collisions do the rest, so a body settles where the pull and
  its drag balance (about `strength / drag`). Measured in a world: a pull of 100 brings a bibite to about 50 u/s and a pellet, whose drag is about 12 a second, to about
  8.5 u/s after a second; 500 gives about 230 and 42. (Nothing moves faster than 400 u/s, whatever the strength.)
* **Falloff**: a switch, **Constant** (the same strength all the way to the edge of the range) or **Fading** (from the start: full at the tower, fading in a straight line to
  nothing at the edge, so there is no step where the range ends). Inside 2 u of the tower nothing acts (there is no direction to a point that is on it).
* **Bibites**, **Pellets**, **Eggs**: a switch for each kind of body it acts on (all on from the start). A corpse is a body and counts as a bibite; a pellet that a mouth
  holds is left alone. A tower of pellets only is how to build a planet without disturbing the bibites, one of bibites only keeps them in a zone without moving the food.
* The line at the bottom says what it is doing ("Pulling 28 bodies", "Nothing in its range") or why it does nothing (the strength is 0, every switch is off).

* A simulation change, so a switch: `Feature::ForceTower` (`force_tower`). Settings > **Towers & Streams** has the switch **Force Towers**: on from the start (with no force tower
  in the world nothing changes either way), off = the towers do nothing and are drawn faint (their panel says so). It draws nothing from the world's random numbers, so a
  world without a tower is untouched (the pinned runs hold that) and one with a tower runs the same way twice. The Towers page lost its two "where to place" lines for
  one ("Placer panel (bottom left), the top buttons") to make room for the third switch.
* Not in a save: the towers are kept in `force_towers/<save name>.json` of the state folder the way the gene towers and vents are (their settings, position and the zone
  by its name; not what they are doing now).
* Costs: each tower visits every body of the kinds it acts on once a tick and moves those inside its range (a pellet that is pulled no longer sleeps, so a big heap of
  pulled pellets costs more than a heap at rest); with no tower there is no cost.
* Not done: another shape of falloff (inverse square), forces on pheromone spots (they are not bodies), a tower that moves, towers acting on each other.
* The code: `crates/bibites-core/src/force_tower.rs` (the tower, its falloff and kick), its `Tool::Force` variant in `tools.rs`, `World::step_force_towers` (after the pellet
  producers and the vents, before the bodies move; since 2026-11-25 the tick steps the force towers, the whirlpool towers and the streams in one pass, `fields.rs`, and this
  is that pass for the force towers alone), checkpoint tag 5, `tower_panel.rs` (the panel), `improved_toolbar.rs` (the placer button), `app/force_tower_actions.rs`
  (the files), `improved_settings.rs` (the Towers page), `render.rs` (the tower and its range drawn).

### Whirlpool tower

A tower that TURNS the things around it in a circle, like a stream bent round a point: clockwise or counterclockwise, as fast as you set. It is the top button of the placer
panel (the bottom-left toolbar's first button; a teal round icon, above the force tower's): click it, click the world to put the tower down (a right click cancels), click the
tower to open its panel:

* **Follow Zone**: the colour killer's dropdown ("None" and the five closest zones): the tower goes where its zone goes.
* **Range**: 10 to 5000 u (300 from the start), how far from the tower it acts. While the tower is selected the range is shown as a faint teal circle.
* **Speed**: 0 to 500 (100 from the start), in u/s per second. Like the force tower's strength and a stream's it is an ACCELERATION along the circle, not a force: a heavy body
  and a light one gain the same speed, and the world's own drag sets the speed a body settles at (about `speed / drag`): a pellet, whose drag is high, goes round slowly, a bibite
  faster. Measured at 300 u from the tower: after 4 s a speed of 100 has taken a bibite 49 degrees round and a pellet 6 (a pellet is slow: it needs 3 to 5 times the speed to go as
  fast); 500 gives 169 and 22.
* **Direction**: a button that says the way it turns now, **Counterclockwise** (from the start) or **Clockwise**, as the world is seen on the screen; a click turns it the other way.
* **Falloff**: the force tower's switch, **Constant** (the same speed all the way to the edge of the range) or **Fading** (from the start: full at the tower, fading in a straight
  line to nothing at the edge). Inside 2 u of the tower nothing acts (a point on the tower is on no circle).
* **Bibites**, **Pellets**, **Eggs**: a switch for each kind of body it acts on (all on from the start). A corpse is a body and counts as a bibite; a pellet that a mouth holds is
  left alone.
* The line at the bottom says what it is doing ("Turning 28 bodies", "Nothing in its range") or why it does nothing (the speed is 0, every switch is off).
* It also **holds** the bodies on their circles: a push along the circle alone would fling a moving body off it (it has momentum and goes straight on), so a bibite would spiral out of
  the whirlpool within seconds. The tower adds the inward acceleration a body needs to turn with the speed it has along the circle (`v^2 / r`, in the same proportion as the
  push, and never more than takes the whole of that speed in one tick). A body that goes round stays at about its distance (a bibite, which also swims on its own, went from 300 u
  to 328 u in 4 s at a speed of 100; a pellet stays put to the unit); a body that swims straight outwards is not stopped and leaves when it is out of range.
* A simulation change, so a switch: `Feature::Whirlpool` (`whirlpool`). Settings > **Towers & Streams** has the switch **Whirlpool Towers**: on from the start (with no whirlpool
  tower in the world nothing changes either way), off = the towers do nothing and are drawn faint (their panel says so). It draws nothing from the world's random numbers, so a
  world without one is untouched (the pinned runs hold that) and one with a whirlpool runs the same way twice. The page has one line ("Placing") in the place of the two of before,
  to make room for the fifth switch.
* Not in a save: the towers are kept in `whirlpool_towers/<save name>.json` of the state folder the way the other towers are (their settings, position and the zone by its name; not
  what they are doing now).
* Costs: the towers and the streams are stepped together, in one pass over the world (`fields.rs`, 2026-11-25): a grid over the fields' boxes tells, for each body, the few
  fields that can reach it, so a tick costs one look at each body plus the work for the bodies a field reaches (a pellet that is turned no longer sleeps), not a walk over the
  whole world for each field. With no field there is no cost. The results are, bit for bit, those of the separate steps they replace (`docs/BENCHMARKS.md`, "Streams, force towers
  and whirlpool towers": twelve fields over the 55,000 pellets of Big Pellet went from 4.7 to 0.9 ms a tick, thirty from 12.7 to 1.3).
* Drawn as the tower's base disc with a teal ring and two arrowheads on it that point the way it turns.
* The code: `bibites-core/src/whirlpool.rs` (the tower, the push and the hold), `world.rs` (`step_whirlpools`), `fields.rs` (the pass all fields share; `kick_bodies` stays in the tests as the old reference), `tower_panel.rs` (`Kind::Whirl`), `app/whirlpool_actions.rs`,
  `improved_toolbar.rs` (the button), `render.rs` (the tower and its range). For pictures: `--whirlpool X,Y[,SPEED[,RANGE]][,clockwise|counterclockwise|cw|ccw][,constant|fading][,bibites|pellets|eggs ...]`.
* Not done: a pull or a push added to the turn (a force tower on the same spot does it), a whirlpool that moves, one that spirals in or out by itself, a speed that changes with time.

### Streams (a section of the Zones tab)

A **stream** is a band of the world that pushes everything inside it the same way: you give it a **start** and a **finish** (two points, in the world's x and y), a **width**
and a **strength**, and the bibites, eggs and pellets inside it are pushed in the direction from the start to the finish. It is drawn as a transparent blue band with a blue
border (and a small arrowhead at the finish, so the way it runs can be seen).

* Where: the world's parameters panel (in a world), the **Zones** tab, a new section **Streams** under Zone Groups, with the same header as the
  other lists: a `+` that makes a stream across the middle of what the camera shows (from left to right, 100 wide, strength 100) with its card open, the title, and a "remove
  all" button (it asks first, as the zones' does; greyed out while there are none). Each stream is a card like a zone group's: closed it shows "Stream 1" and its bar, open it
  has a delete button (the stream goes at once) and six sliders in two columns: **Start X**, **Start Y**, **Width** (left) and **End X**, **End Y**, **Strength** (right). A click on a
  value types a number into it, as everywhere else.
* The band is the rectangle that has the line from the start to the finish as its middle line and the width across it: it begins at the start and ends at the finish (a body
  past either end is outside it). The push is the same everywhere in the band. Two streams that overlap push by the sum of their pushes. A stream whose start is on its finish
  has no direction, so it pushes nothing and is not drawn.
* **Strength** is 0 to 500 (100 from the start), in u/s per second, an ACCELERATION like the force tower's, not a force: a heavy body and a light one gain the same speed.
  Every tick the stream adds `strength * dt` to the velocity of each body inside it, towards the finish; the world's own drag and collisions do the rest, so a body in a stream
  settles at a speed of about `strength / drag` (a pellet's drag is much higher than a bibite's, so the same stream carries a bibite fast and a pellet slowly: raise the
  strength for pellets). **Width** is 10 to 3000 u (100 from the start, a log slider). The coordinates run over the whole world (`-size` to `size`); a typed value past an end
  is held to it.
* Bibites (the dead ones too), eggs and pellets that no mouth holds are pushed; a pellet at rest wakes up when a stream pushes it.
* A simulation change, so a switch: `Feature::Streams` (`stream`). Settings > **Towers & Streams** has the switch **Streams**: on from the start (with no stream in the world
  nothing changes either way), off = the streams stay in the world, drawn faint, and push nothing. A stream draws nothing from the world's random numbers, so a world without
  one is untouched (the pinned runs hold that) and one with a stream runs the same way twice. The page was called "Towers" and holds the switches of the towers as well.
* Not in a save: the streams are kept in `streams/<save name>.json` of the state folder (the way the towers and the zone colours are), written when the world is saved and
  beside each autosave, read when a save is loaded. The title menu's scenario editor has no Streams section (a scenario is a game file).
* They are drawn with the zones: the View panel's zones switch hides them too.
* The code: `bibites-core/src/stream.rs` (the band, the push), `world.rs` (`step_streams`), `fields.rs` (the pass all fields share), `dyn_panel/stream_cards_ui.rs` (the cards), `app/stream_actions.rs` (adding,
  editing, deleting, keeping), `zone_editor.rs` (the sliders' ranges), `render.rs` (`Primitive::Quad`, `push_streams`). For pictures: `--stream X1,Y1,X2,Y2[,WIDTH[,STRENGTH]]`
  (repeatable; the cards open) with `--dyn-tab 1`.
* Not done: a curved stream, a strength that fades along the band or across it, dragging the two ends in the world with the mouse, streams in the scenario editor.

### Hide Objects (a hotkey with a key of your own)

**V** (from the start) hides the world's objects and shows them again: bibites, eggs, pellets, placed towers and the particles (pheromone and blood), and what hangs on a
bibite (the selected one's field of view, the "Show Targets" labels, its status bars). What stays is the interface and the world's floor: every panel and graph, the
tooltips, the background, the grid and the zones, and a tower while it is being placed. A way to watch the numbers without paying to draw the crowd.

* It is a way of looking, not a setting: not stored (a window starts with the objects shown), and the simulation does not notice (nothing in a tick reads it; the scene
  builder does not visit the hidden objects, though the per-frame upkeep of the bibites' tails still does). It is not a simulation switch for that reason.
* Clicks still work on what cannot be seen (a click on the place a bibite is selects it, `R` still picks one): only the drawing is off. A pellet that is not drawn has no
  tooltip. A hotkey like the game's: it waits while a text field has the keyboard, a dialog or the pause menu is open, and it ignores the key with Ctrl or Alt held
  (Ctrl + V is the tag paste).
* **The key is the player's.** Settings > **Extra Hotkeys** (the button under Towers; title menu or the cog in a world): a button with the key on it (click it, then
  press the new one; Escape or a click elsewhere cancels, as does going to another page), "Reset" for the default, and a line that says what to do or what just
  happened. A key the game already has a job for (WASD, R E G O Z K, H, 1 to 4, F1 to F5, F9, F12) is refused and the line says what it does; the wait goes on. Letters,
  digits, F6 to F8 and F10 to F11, Tab, the punctuation keys and the keypad's digits can be used. The keys are in `improved_keys.txt` of the state folder
  (`action=key` lines, `hide_objects=V`; a hand-written line with a refused key is ignored).
* While a key is waited for, that key press is the page's alone (Escape does not also close the screen, a letter is not a hotkey).
* Not done: a check box for it in the View panel, a hint on the screen while hidden, hiding the selection by clicks.
* The code: `crates/bibites-view/src/improved_keys.rs` (the keys, the wait, the file), `SceneOptions::objects` and its uses in `build_scene` (`render.rs`),
  `app/hidden_objects.rs` (the hotkey), `FrameInput::toggle_objects` (read in `main.rs::gather_input`), `improved_settings.rs` (the Extra Hotkeys page).
  The simulation thread builds its picture with the same option, so the hotkey needs no world (`App::light_input`).

## Where things live (so the editions do not mix)

* **Settings and the port's own state** (`user_settings.json`, challenge progress, achievements) are in `./userdata` of the folder the program is started from, or in `RUSTIBITES_DATA_DIR` when that is set. The Improved shortcut starts in `Z:\RUSTibites-improved`, so Improved has its own `userdata` (empty at first). `RUSTIBITES_DATA_DIR` is not set on this machine; do not set it globally, it would make both editions share.
* **Worlds, autosaves, scenarios and bibite files** go into the installed game's own folder (`%USERPROFILE%\AppData\LocalLow\The Bibites\The Bibites`) in both editions, by design: the original game lists what the port saves. They are shared between Faithful, Improved and the game. Set `RUSTIBITES_DATA_DIR` for a run to keep one edition's saves apart.
* **Build output** is per folder (`target_alpha`, `target_light`, ... inside each). The Improved folder started with a copy of Faithful's `target_alpha` and `target_light` so that the registry crates did not have to be compiled again; the workspace crates rebuild on their own paths.
* **Shared by junction, read-only use:** `Z:\RUSTibites-improved\Here` (the decompiled game source, the ground truth for behaviour) and `Z:\RUSTibites-improved\testdata` (the test saves) point at the Faithful folder's. A tool that writes into `testdata` writes there. `target\pyi` (the frozen art extractor used by `build_alpha.ps1`) is a copy.
* **Not carried over:** the `sdf-text-outline-WIP` stash (it belongs to the repository, `git stash list` shows it from either folder) and the two old detached worktrees in `AppData\Local\Temp` from earlier performance work (`git worktree list`).

## Day to day

* Work in `Z:\RUSTibites-improved` on the `improved` branch. Build, test and package exactly as for Faithful (the same RAM guard and light/alpha scripts, pointed at that folder).
* Desktop: `RUSTibites Improved.lnk` runs `Z:\RUSTibites-improved\target\release\rustibites-view.exe` with that folder as its start-in directory; the Faithful shortcuts are untouched.
* Tester package: `tools\release\build_alpha.ps1 -Exe target_alpha\perf\rustibites-view.exe -SkipFreeze` (the static-CRT build, like Faithful's) makes `target\alpha-package\RUSTibites-Improved-alpha-<date>-<hash>.zip`. It is the plain package plus `IMPROVED.txt` (`tools\release\alpha`): what the extras are, where their files are, and about the title menu's moving background; keep it true when an extra changes. That background is the game's own video played through ffmpeg, which is not in the package: `play.ps1` runs `ensure_ffmpeg.ps1` (a file of the Improved package only) at every start, which does nothing when ffmpeg is found (the viewer's own places: `RUSTIBITES_FFMPEG`, the PATH, `C:\ffmpeg\bin`, Program Files, winget's packages) and otherwise installs it ONCE with `winget install Gyan.FFmpeg.Essentials` (a small download, for the current user, no administrator rights; it says so on the screen and the game starts whatever happens). What it did is in `ffmpeg_install.txt` (a failed try is repeated after a day, a missing winget after a month); `Play.bat -NoFfmpegInstall`, an empty `no_ffmpeg_install.txt` next to Play.bat or the variable `RUSTIBITES_NO_FFMPEG_INSTALL` stop it. Tested against a fake winget (every branch); the real install was not run on this machine. The package holds no game art; the first run builds it from the player's own copy of the game.
* After fixes on `main`: `git -C Z:\RUSTibites-improved merge main`, build, run the tests.
* Removing the Improved folder cleanly: `git worktree remove Z:\RUSTibites-improved` (the branch stays until `git branch -d improved`).
