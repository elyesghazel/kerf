# kerf roadmap

The goals, in priority order: **open and save instantly, edit without lag on a
small laptop, let agents do the tedious cutting**. Rendering speed matters
less, and heavy renders can be sent to another machine.

## Decisions so far

- **Multiple projects = multiple windows (processes), not tabs.** Each
  `kerf-studio` window has its own process (Ctrl+N, File › Open in new
  window). This isolates crashes, uses all the cores, and the OS window manager
  handles the layout (Hyprland tiling works well for this). That is cheap
  because opening is near-instant: the project is a small JSON edit list and
  every asset is cached by file. Revisit only if moving clips *between*
  projects becomes common (that is just copy/paste of `Command` JSON across
  windows, see "Clipboard" below).
- **ffmpeg as a subprocess, not linked.** The build never breaks when the
  system jumps FFmpeg versions, and a decoder crash can't kill the UI.
- **One engine for preview, CLI and export.** Whatever an agent sees with
  `kerf frame` is exactly what gets exported.
- **The project file is the API.** Agents and the studio meet at the `.kerf`
  file. Edits from either side arrive as undoable steps.

## Next (v0.3): smooth on the laptop

- [ ] **Prefetch the next clip's decoder** about 1 s before a cut, so playback
      doesn't hitch at clip boundaries (today a cut costs one ffmpeg spawn,
      roughly 40–80 ms).
- [ ] **Frame cache:** an LRU of rendered preview frames around the playhead,
      so scrubbing back and forth over the same region is free.
- [ ] **Auto-proxy policy:** proxy anything above 1080p or in HEVC on import,
      with a per-machine setting.
- [ ] **Audio scrubbing:** short audio snippets while dragging the playhead.
- [ ] **Clipboard:** Ctrl+C/V clips as `Command` JSON (also works between
      windows and from agents).
- [ ] **Duplicate, nudge** (`,` `.` move a clip by one frame), **roll/slip**
      trims, and ripple trims (Q/W like Resolve).
- [ ] **Marquee selection** and "select all clips after the playhead".
- [ ] **Clip speed** (0.25×–4×, with pitch-corrected audio).
- [ ] **Welcome screen** with recent projects and thumbnails.

## Then (v0.4): the agent loop

- [ ] **Transcripts:** whisper.cpp via a subprocess, stored per media item in
      the cache. Agents cut by words (`kerf cut-words "um" "uh"`, "keep the
      sentences about X"), and the studio shows a text panel where you can
      delete words to cut them.
- [ ] **Auto captions** from the transcript, using title styles, burned in or
      exported as SRT.
- [ ] **Beat detection** on music tracks, so cuts can snap to beats.
- [ ] **Agent chat panel** in the studio, running Claude through the MCP
      tools on the open project, with the edit list visible as it happens.
- [ ] **Edit decision log:** every `Command` with who made it (you or an
      agent), shown as a history panel you can scrub through.
- [ ] **`kerf review`:** renders a low-res preview and a contact sheet and
      posts them to the lab page so you can approve a cut from the phone.

## Later (v0.5): finishing

- [ ] **Keyframes** for transform, opacity, volume and grade, with ease curves.
- [ ] **Transitions:** cross-dissolve, dip to colour and wipes as real
      transition objects (today: fades + overlap on V2).
- [ ] **LUTs:** `.cube` import per clip and per timeline.
- [ ] **GPU compositor:** move the compositing to wgpu shaders so 4K preview
      runs at full rate on the iGPU. Grading is LUT-based today, so it ports
      cleanly.
- [ ] **Audio:** EQ, a compressor, loudness normalise to −14 LUFS on export,
      ducking music under speech.
- [ ] **Masks & blur** (blur a face or a number plate) and simple tracking.
- [ ] **Text:** more fonts (bundled per project, so remote renders match),
      outlines, animated presets (pop, typewriter).
- [ ] **Nested timelines / compound clips.**

## Remote rendering

- [x] `kerf export --on <ssh-host>` and the "Render on" field in Deliver:
      media is rsynced into `~/.cache/kerf/remote-media` on the host once,
      the project paths are rewritten, the render runs there, and the file is
      pulled back.
- [ ] **Device setup wizard:** `kerf devices add pc` checks SSH, installs
      kerf on the host (cargo or a prebuilt binary), and benchmarks it. The
      Deliver page then lists your devices with their speed.
- [ ] **Wake-on-LAN** the PC before a remote render (the VPS already has WoL
      for the PC), and let it sleep again afterwards.
- [ ] **Split renders:** cut the timeline into chunks, render them on several
      machines in parallel, and stitch with stream copy.
- [ ] **Render queue** that survives closing the studio (a small daemon), with
      a ntfy push when it's done.
- [ ] **Hardware encode on the PC** (NVENC on the RTX 3060) as a preset.

## Platform

- [ ] Packages: an AUR `kerf-bin`, a `get.sh` installer, and a `.desktop`
      entry with an icon.
- [ ] macOS and Windows builds (nothing Linux-only in the core; the VA-API
      preset is Linux-only).
- [ ] Accessibility labels on custom widgets (the UI test currently turns this
      check off).
