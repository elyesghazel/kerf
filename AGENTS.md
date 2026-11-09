# Cutting video with kerf: a guide for AI agents

kerf is a timeline video editor. A project is a small `.kerf` JSON file: a media bin plus tracks of clips. It references media on disk and never embeds it. You edit a project with the `kerf` CLI or the `kerf mcp` server. Both run the same operations the studio GUI uses, and every edit is saved atomically and can be undone.

## Workflow

```sh
export KERF_PROJECT=cut.kerf                  # or pass -p cut.kerf to every command
kerf new cut.kerf --preset vertical           # 1080p | 720p | 4k | vertical | square | 4:5
kerf import a.mp4 b.mov music.mp3 --append    # probe + add to bin; --append also lays them end to end
kerf info                                     # READ THIS: track/clip ids, spans, source in-points, links
kerf silences 5          # silent stretches in media 5 (SOURCE times)
kerf scenes 5            # hard cuts / scene changes (SOURCE times)
kerf loudness 5          # LUFS + true peak
# ... edit ...
kerf contact -o sheet.png                     # 12-frame overview of the whole edit (look at it!)
kerf frame --at 12.5 -o f.png                 # one composited frame, pixel-identical to export
kerf export -o out.mp4                        # --codec h264|h265|vaapi|prores, --from/--to, --quality
kerf export -o out.mp4 --on pc                # render on another SSH/Tailscale host (needs kerf there)
```

Add `--json` to any command for machine-readable output. Errors exit non-zero.

Times can be given as `12.5`, `12.5s`, `1:02.5` or `00:01:02:15` (frames at the project fps). Timeline times and source times are different. `silences`/`scenes` report **source** times inside a media file. To get a timeline time, take the clip's `start + (source_t - src_in)` from `kerf info`.

## Editing commands

| Goal | Command |
| --- | --- |
| put media at a time | `kerf place <media> --at 10 [--in 3 --dur 5] [--track <id>]` |
| split a clip (and its linked audio/video) | `kerf split <clip> --at 7.2` |
| split every track | `kerf blade --at 7.2` |
| remove a range from everything, closing up | `kerf cut --from 4 --to 7` (`--no-ripple` leaves a gap) |
| keep only some ranges (highlight reel) | `kerf keep 3-8 15.5-21 40-44` |
| drop silences automatically | `kerf auto-cut-silence [--db -35 --min 0.5 --pad 0.15] [--dry-run]` |
| delete clips | `kerf delete 12 13 [--ripple]` |
| close empty gaps | `kerf close-gaps` |
| move clips | `kerf move 12 --by -1.5 [--tracks 1]` |
| trim an edge | `kerf trim 12 --start 3.0` / `--end 9.5` |
| clip properties | `kerf set 12 volume=0.5 fade_in=0.3 grade.saturation=1.2 transform.scale=1.1` |
| title | `kerf text "Day 1" --at 0 --dur 3 --bold --size 0.1 --y 0.5 --bg '#00000099'` |
| markers | `kerf marker add --at 30 "drop"`, `kerf marker rm <id>` |
| tracks | `kerf track add video`, `kerf track set 3 mute=true volume=0.8`, `kerf track rm 4` |
| output format | `kerf format --preset vertical --fps 30` |
| undo / redo | `kerf undo`, `kerf redo` (history in `<project>.kerf-history.json`) |
| faster preview | `kerf proxy all` (540p proxies; export always uses originals) |

`set` keys: `volume opacity fade_in fade_out fade name start source_in duration color`, `grade.{exposure,brightness,contrast,saturation,gamma,temperature,tint,vignette}`, `transform.{scale,x,y,rotation,crop_left,crop_right,crop_top,crop_bottom,flip_h}`, `text text.{size,x,y,color,background,bold,shadow}`, and `grade=reset` / `transform=reset`. Text `size` is a fraction of the frame height. `x`/`y` are the text centre on a 0..1 scale.

How the model works:
- Video tracks stack, so a higher track (V2) covers a lower one (V1).
- A file that has both picture and sound becomes two clips, one on a video track and one on an audio track, sharing a `link` group. Splitting, moving and trimming either clip also changes its partner.
- Clips on the same track cannot overlap. An edit that would cause an overlap is rejected and leaves the project unchanged.

## Raw commands (`kerf apply`)

For edits the CLI doesn't cover, send `Command` JSON. A single object or an array is applied as one undo step, and the whole step fails if any part of it fails:

```sh
kerf apply - <<'EOF'
[
  {"op": "move_clip", "track": 1, "clip": 12, "new_start": 4000000},
  {"op": "trim_clip", "track": 1, "clip": 12, "start": 4000000, "source_in": 1500000, "duration": 2500000},
  {"op": "update_track", "id": 3, "name": "A1", "muted": false, "hidden": false, "locked": false, "volume": 0.7},
  {"op": "rename", "name": "Final cut"}
]
EOF
```

Raw commands use **microseconds** for times. The ops are `add_media remove_media update_media add_clip remove_clip move_clip trim_clip update_clip set_track_clips add_track remove_track update_track add_marker remove_marker set_format rename batch replace`. See `crates/kerf-core/src/command.rs`. `kerf info --json` dumps the full project in the same schema.

## Live with the GUI

The studio (`kerf open` or `kerf-studio cut.kerf`) watches the project file. When an agent saves an edit, the open editor reloads it right away, and that reload is one undoable step in the GUI. A human can watch an agent cut and press Ctrl+Z to reject an edit. Edits the human makes in the studio are autosaved to the same file, so run `kerf info` again before you edit.

## MCP

```sh
claude mcp add kerf -- kerf mcp
# or with a default project:
claude mcp add kerf -e KERF_PROJECT=/path/cut.kerf -- kerf mcp
```

The MCP tools mirror the CLI: `info new_project import place append split blade delete cut_range keep_ranges close_gaps move trim set_clip add_text add_marker set_format undo redo silences scenes loudness auto_cut_silence render_frame contact_sheet export apply_commands`. Each tool accepts an optional `project` path. `render_frame` and `contact_sheet` return PNG images, so you can check your edit visually.

## Tips

- Run `kerf info` before you edit and again after. Clip ids change when clips are split or cut.
- Cut from the end of the timeline towards the start, or use `keep`/`cut`, which handle ripple for you.
- Cut speech on silences, not on fixed times. Leave `--pad` around 0.1-0.2s so words don't get clipped.
- Look at a contact sheet before you export. It's cheap.
