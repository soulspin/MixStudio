/// Every symbol in a UI string literal must exist in the bundled egui fonts; missing glyphs render as "□".
#[test]
fn ui_strings_only_use_glyphs_from_bundled_fonts() {
    let sources = [include_str!("../app.rs"), include_str!("transition_workbench.rs"), include_str!("comparison.rs"), include_str!("library_ui.rs"),
        include_str!("levels_ui.rs"), include_str!("harmony_ui.rs"), include_str!("sample_ui.rs"), include_str!("stem_volume.rs"), include_str!("clip_fades.rs"),
        include_str!("persistence.rs"), include_str!("library_drop.rs"), include_str!("../model.rs"), include_str!("../harmony.rs"), include_str!("../automix.rs"), include_str!("../library.rs"), include_str!("../library/browse.rs")];
    let ctx = egui::Context::default(); let _ = ctx.run(Default::default(), |_| {});
    let mut missing = std::collections::BTreeSet::new();
    for src in sources { for line in src.lines().take_while(|l| !l.starts_with("#[cfg(test)]")).filter(|l| !l.trim_start().starts_with("//")) {
        for (k, part) in line.split('"').enumerate() { if k % 2 == 1 { for c in part.chars().filter(|c| !c.is_ascii()) {
            if !ctx.fonts(|f| f.has_glyph(&FontId::proportional(14.0), c)) { missing.insert(c); }
        } } }
    } }
    assert!(missing.is_empty(), "glyphs missing from the UI fonts: {missing:?}");
}
