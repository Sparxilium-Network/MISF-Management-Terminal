# Fonts embedded by egui

MISF enables eframe's `default_fonts` feature, so `misf` embeds these fonts from
the `epaint_default_fonts` 0.37.0 crate. The license files here are verbatim
copies from that crate's `fonts/` directory.

| Font file | Use | License |
|---|---|---|
| `Ubuntu-Light.ttf` | Default proportional UI font | Ubuntu Font Licence 1.0: `UFL.txt` |
| `Hack-Regular.ttf` | Default monospace font | MIT (Hack), Bitstream Vera License, DejaVu public domain changes: `Hack-Regular.txt` |
| `egui-icons.ttf` | Five glyphs from emoji-icon-font, for `egui::special_emojis` | MIT: `emoji-icon-font-mit-license.txt`; subset details in `egui-icons.txt` |

Noto Emoji and the full emoji-icon-font are **not** embedded: they need the
`monochrome_emoji_fonts` feature, which MISF does not enable. The `system_fonts`
feature loads fonts installed on the user's system at run time; those are not
distributed with MISF.

When upgrading eframe, compare this list with `epaint/src/text/font_definitions.rs`
and recopy the files if the fonts or their license texts change.
