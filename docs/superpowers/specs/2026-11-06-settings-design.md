# Settings design

## Intent

Nobody opens `~/.config/ari/config.toml`. Today it is written by `ari setup` with no comments, half the keys are missing, colours are fixed, and edits only apply after a restart. The owner wants ARI's config to feel like a professional app's settings: open it from the panel or hub, flip things, and see the result straight away.

## Goals

1. **A settings screen** in `ari watch`, opened with `s` from the panel, the hub or the `--all` list. Every change applies live and is saved to `config.toml` at once.
2. **Colour themes**: a few ready-made themes to switch between, plus overriding a single colour (owner's choice: "themes plus single-colour tweaks").
3. **Feature switches**: turn panel sections and statusline segments on and off. The task checklist is one of them, and it stays on by default.
4. **Live reload**: hand edits to `config.toml` apply in open `ari watch` windows without a restart; the statusline already reloads on every run.
5. **No silent failure**: a broken `config.toml` is reported (settings screen, `ari doctor`), and is never overwritten by the settings screen.

## Non-goals

- Editing `[chain]` and `[prices]` in the screen. They stay file-only; the screen's footer says so.
- Hub layout options (which hub panes show).
- Keeping hand-written comments when the screen saves (would need a new crate, `toml_edit`). The file gets a fixed header comment instead.
- Changing `ari demo`'s look. It replays with defaults.

## Anti-goals

- The default theme changes nothing on screen. Existing snapshots stay identical except where the new `s settings` key hint appears.
- The chained statusline's output is never recoloured.
- A save never destroys a file ARI could not read.

## Decisions (my calls; the owner skipped the remaining questions)

### Config keys (new)

```toml
theme = "default"           # default · soft · vivid · colorblind · mono
hide = ["cost"]             # any feature id below; now also panel sections

[colors]                    # override one colour; name, "#rrggbb" or 0–255
good = "#5fd787"            # good warn bad accent highlight info muted bar
```

### Colour roles

ARI keeps drawing with its eight base colours. A palette maps each base colour to a role colour at the very end of drawing:

| Role | Base colour | Used for |
|---|---|---|
| good | Green | healthy bars, passes, done |
| warn | Yellow | warnings, the lead agent |
| bad | Red | errors, critical bars |
| accent | Cyan | working, task bars, agents |
| highlight | Magenta | messages |
| info | Blue | read tools |
| muted | DarkGray | faint text |
| bar | Indexed(236), as background | the hub's top and bottom bars |

Themes, in cycle order:

| Theme | good | warn | bad | accent | highlight | info | muted | bar |
|---|---|---|---|---|---|---|---|---|
| default | Green | Yellow | Red | Cyan | Magenta | Blue | DarkGray | 236 |
| soft | 114 | 179 | 167 | 116 | 176 | 110 | 243 | 236 |
| vivid | LightGreen | LightYellow | LightRed | LightCyan | LightMagenta | LightBlue | Gray | 234 |
| colorblind | 39 | 220 | 208 | 255 | 175 | 111 | 243 | 236 |
| mono | Reset | Reset | Reset | Reset | Reset | Reset | DarkGray | 236 |

Numbers are 256-colour indexes. An unknown theme name means `default`; an unreadable colour means the theme's colour. Both are reported by `ari doctor`.

### Feature ids (the `hide` list)

One id switches the same feature everywhere it appears.

| id | Panel | Statusline |
|---|---|---|
| tasks | task checklist / plan bar in the header | tasks segment |
| agents | agents section | agents segment |
| errors | errors section | errors segment |
| usage | usage section | – |
| session | session section | – |
| turns | turns section | – |
| conflicts | – | conflicts segment |
| now | – | now-doing segment |
| tests | – | tests segment |
| ctx, 5h, 7d, cost, tokens | – | those segments |

All are on by default.

### The settings screen

`s` opens it from anywhere in `ari watch`. Left column: the list. Right column, when the window is at least 110 wide: the live panel for the session being followed, drawn with the edited config.

```
 ARI settings                                    ~/.config/ari/config.toml
 LOOK                                   │ ╭─ ARI ───────────────── 12:41 ─╮
 ▸ theme              ‹ default ›       │ │ ● WORKING · auth refactor    │
   face               off               │ │ ▰▰▰▰▱▱ 3/5 tasks             │
   ascii glyphs       off               │ ...
   statusline row     top               │
   statusline width   auto              │
 COLOURS                                │
   good               ■ theme           │
 ...
 FEATURES
   task checklist     on
 ...
 ALERTS
   bell               off
   context warn       60%
 ...
 ↑↓ move · ←→ change · d default · esc back · chain and prices: edit the file
```

- Rows (group, label, key): LOOK: theme, face, ascii glyphs, statusline row (`position`), statusline width (`max_width`, 0 shows `auto`). COLOURS: the eight roles, each with a swatch. FEATURES: every feature id, label `<id> · <what>` except tasks, labelled `task checklist`. ALERTS: bell, context warn, context critical, stuck after, compact at. DATA: keep logs.
- Keys: `↑↓`/`jk` move, `←→`/`hl` step the value, `⏎`/space step forward, `d` resets the row to its default, `esc`/`s` close, `q` quits.
- Steps: theme cycles; booleans flip; position flips; width 0 → 40 → 60 … by 20, max 200; ctx warn ±5 within 5..crit−5; ctx crit ±5 within warn+5..100; stuck after ±1 within 2..10; compact at ±5 within 50..100; keep logs ±1 within 1..90. A colour cycles `theme` → red green yellow blue magenta cyan white gray darkgray lightred lightgreen lightyellow lightblue lightmagenta lightcyan → `theme`; a hand-typed value steps to `theme`.
- The list scrolls to keep the selected row visible. Below 40×10 it says `settings need 40×10`.
- If `config.toml` can't be read, the first row is the error in red and every edit is refused with `fix config.toml first (or delete it)`.
- A failed save shows `couldn't save: <error>` in the footer.

### Saving and reloading

- The screen saves after every change, atomically (`paths::write_atomic`), with this header:
  `# ARI settings. Edit here or press s in ari watch; changes apply live.` / `# Every key: https://github.com/Izuulo/ari#config`
- A `--ascii` flag on the command line is never written to the file.
- `ari watch` checks the file's modification time on each refresh and reloads it when it changes. A reload that fails keeps the last good config and shows the error in the settings screen.

### Where it's reported

- `ari doctor` gets a `config` check: no file (pass, defaults), file fine (pass), parse error (problem, fix: "fix that line, or delete the file to use defaults"), or a list of unknown theme / colour role / colour value / hide id (problem).
- The panel's key hint and the hub's bottom bar list `s settings`.
- README's Config section documents the new keys, the themes table, and the feature ids.

## Left to the builder

Exact widths of the two settings columns, swatch glyph in ASCII mode, and the wording of each feature's short description.
