//! kerf studio: a light, fast video editor.

mod app;
mod assets;
mod engine;
mod export_ui;
mod panels;
mod theme;
mod timeline;
mod viewer;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let title = path
        .as_ref()
        .and_then(|p| p.file_stem())
        .map(|s| format!("kerf · {}", s.to_string_lossy()))
        .unwrap_or_else(|| "kerf".into());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_app_id("kerf")
            .with_inner_size([1480.0, 920.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true)
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-128.png")).unwrap_or_default()),
        // Only repaint when something changes: idle costs ~0% CPU.
        run_and_return: false,
        ..Default::default()
    };
    eframe::run_native("kerf", options, Box::new(move |cc| Ok(Box::new(app::KerfApp::new(cc, path)))))
}

/// Headless screenshots of the real app (software Vulkan via lavapipe):
/// `KERF_SHOT_PROJECT=demo.kerf KERF_SHOT_OUT=dir cargo test -p kerf-app shots -- --ignored`
#[cfg(test)]
mod shots {
    use std::time::Duration;

    use crate::app::{KerfApp, Workspace};

    fn settle(h: &mut egui_kittest::Harness<'_, KerfApp>, ms: u64) {
        let end = std::time::Instant::now() + Duration::from_millis(ms);
        while std::time::Instant::now() < end {
            h.step();
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    #[test]
    #[ignore]
    fn shots() {
        let project = std::env::var("KERF_SHOT_PROJECT").expect("KERF_SHOT_PROJECT");
        let out = std::path::PathBuf::from(std::env::var("KERF_SHOT_OUT").expect("KERF_SHOT_OUT"));
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1480.0, 920.0))
            .with_pixels_per_point(1.0)
            .with_accessibility_check(false)
            .build_eframe(|cc| KerfApp::new(cc, Some(project.into())));
        settle(&mut h, 2500);
        {
            let app = h.state_mut();
            app.seek(kerf_core::Time::from_secs_f64(2.4));
            let ids: Vec<_> = app.project().timeline.clips().map(|(_, c)| c.id).collect();
            app.select_with_links(ids[2], false);
        }
        settle(&mut h, 2500);
        let save = |h: &mut egui_kittest::Harness<'_, KerfApp>, name: &str| {
            let img = h.render().expect("render");
            img.save(out.join(name)).unwrap();
        };
        save(&mut h, "edit.png");
        h.state_mut().workspace = Workspace::Color;
        settle(&mut h, 800);
        save(&mut h, "color.png");
        {
            let app = h.state_mut();
            app.workspace = Workspace::Deliver;
            app.selection.clear();
            app.seek(kerf_core::Time::from_secs_f64(14.0));
        }
        settle(&mut h, 1500);
        save(&mut h, "deliver.png");
        h.state_mut().workspace = Workspace::Audio;
        settle(&mut h, 600);
        save(&mut h, "audio.png");
        {
            let app = h.state_mut();
            app.theme_mode = crate::theme::Mode::Light;
            app.workspace = Workspace::Edit;
            app.seek(kerf_core::Time::from_secs_f64(2.4));
            let ids: Vec<_> = app.project().timeline.clips().map(|(_, c)| c.id).collect();
            app.select_with_links(ids[2], false);
        }
        settle(&mut h, 1500);
        save(&mut h, "light.png");
    }
}

/// Drives the timeline with real pointer/keyboard events.
#[cfg(test)]
mod interact {
    use std::time::Duration;

    use egui::{Event, Modifiers, PointerButton, Pos2};
    use kerf_core::{ClipContent, Time};

    use crate::app::{KerfApp, Tool};

    type H<'a> = egui_kittest::Harness<'a, KerfApp>;

    fn steps(h: &mut H<'_>, n: usize) {
        for _ in 0..n {
            h.step();
            std::thread::sleep(Duration::from_millis(16));
        }
    }

    fn drag(h: &mut H<'_>, a: Pos2, b: Pos2) {
        h.event(Event::PointerMoved(a));
        steps(h, 2);
        h.event(Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        steps(h, 2);
        for i in 1..=10 {
            let p = a + (b - a) * (i as f32 / 10.0);
            h.event(Event::PointerMoved(p));
            steps(h, 1);
        }
        h.event(Event::PointerButton { pos: b, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        steps(h, 3);
    }

    fn click(h: &mut H<'_>, a: Pos2) {
        h.event(Event::PointerMoved(a));
        steps(h, 2);
        h.event(Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        steps(h, 1);
        h.event(Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        steps(h, 3);
    }

    fn clip_by_name(app: &KerfApp, name: &str) -> kerf_core::Clip {
        app.project().timeline.clips().map(|(_, c)| c.clone()).find(|c| c.name == name).unwrap()
    }

    #[test]
    #[ignore]
    fn timeline_editing() {
        let project = std::env::var("KERF_SHOT_PROJECT").expect("KERF_SHOT_PROJECT");
        // Work on a copy so the demo stays intact.
        let tmp = std::env::temp_dir().join("kerf-interact.kerf");
        std::fs::copy(&project, &tmp).unwrap();
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1480.0, 920.0))
            .with_pixels_per_point(1.0)
            .with_accessibility_check(false)
            .build_eframe(|cc| KerfApp::new(cc, Some(tmp.clone())));
        steps(&mut h, 30);
        let v1 = h.state().project().primary_video_track().unwrap();
        let n0 = h.state().project().timeline.clips().count();

        // 1. Move the big title from 1s to ~6s.
        let title = clip_by_name(h.state(), "NEON RIDE");
        let (tr, _) = h.state().project().timeline.find_clip(title.id).unwrap();
        let y = h.state().tl.lane_y(tr).unwrap();
        let a = Pos2::new(h.state().tl.x_of(Time::from_secs_f64(2.0)), y);
        let b = Pos2::new(h.state().tl.x_of(Time::from_secs_f64(7.0)), y);
        drag(&mut h, a, b);
        let moved = h.state().project().timeline.find_clip(title.id).unwrap().1.start.as_secs_f64();
        println!("title start after drag: {moved:.3}");
        assert!((moved - 6.0).abs() < 0.2, "title moved to {moved}");

        // 2. Trim the end of life.mp4 (ends at 39s) back to 35s.
        let life = clip_by_name(h.state(), "life.mp4");
        let y = h.state().tl.lane_y(v1).unwrap();
        let a = Pos2::new(h.state().tl.x_of(life.end()) - 2.0, y);
        let b = Pos2::new(h.state().tl.x_of(Time::from_secs_f64(35.0)), y);
        drag(&mut h, a, b);
        let lc = h.state().project().timeline.find_clip(life.id).unwrap().1.clone();
        println!("life end after trim: {:.3}", lc.end().as_secs_f64());
        assert!((lc.end().as_secs_f64() - 35.0).abs() < 0.2);
        // Its linked audio was trimmed too.
        let audio = h.state().project().timeline.linked(life.id);
        for (_, id) in audio {
            let c = h.state().project().timeline.find_clip(id).unwrap().1.clone();
            assert_eq!(c.end(), lc.end());
        }

        // 3. Blade tool click on the fractal clip at 6s.
        h.state_mut().tool = Tool::Blade;
        steps(&mut h, 2);
        let x = h.state().tl.x_of(Time::from_secs_f64(6.0));
        click(&mut h, Pos2::new(x, y));
        let n1 = h.state().project().timeline.clips().count();
        println!("clips: {n0} -> {n1} after blade");
        assert_eq!(n1, n0 + 2, "blade splits video + linked audio");

        // 4. Keyboard: select tool, playhead to 15 s, S splits everything there.
        h.key_press(egui::Key::A);
        steps(&mut h, 2);
        h.state_mut().seek(Time::from_secs_f64(15.0));
        h.state_mut().selection.clear();
        steps(&mut h, 2);
        h.key_press(egui::Key::S);
        steps(&mut h, 3);
        let n2 = h.state().project().timeline.clips().count();
        println!("clips after S at 15s: {n2}");
        assert!(n2 >= n1 + 2);

        // 5. Undo everything back to the start, step by step.
        for _ in 0..4 {
            h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
            steps(&mut h, 2);
        }
        assert_eq!(h.state().project().timeline.clips().count(), n0);
        let t = clip_by_name(h.state(), "NEON RIDE");
        assert_eq!(t.start, title.start);

        // 6. An agent edits the file on disk: the studio picks it up.
        let mut p = kerf_core::Project::load(&tmp).unwrap();
        let mut d = kerf_core::Document::new(p.clone());
        d.add_text(kerf_core::TextStyle::new("from agent"), Time::ZERO, Time::from_secs_f64(1.0)).unwrap();
        p = d.into_project();
        std::thread::sleep(Duration::from_millis(1100));
        p.save(&tmp).unwrap();
        steps(&mut h, 40);
        let found = h
            .state()
            .project()
            .timeline
            .clips()
            .any(|(_, c)| matches!(&c.content, ClipContent::Text(s) if s.text == "from agent"));
        println!("agent edit synced: {found}");
        assert!(found);
        h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
        steps(&mut h, 3);
        let gone = !h
            .state()
            .project()
            .timeline
            .clips()
            .any(|(_, c)| matches!(&c.content, ClipContent::Text(s) if s.text == "from agent"));
        assert!(gone, "agent edit is undoable");
    }
}
