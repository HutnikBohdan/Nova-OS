#[cfg(feature = "legacy-monolith-proofs")]
use fontdue::{Font, FontSettings};
#[cfg(feature = "legacy-monolith-proofs")]
use std::{env, fmt::Write as _, fs, path::PathBuf};

fn main() {
    #[cfg(feature = "legacy-monolith-proofs")]
    build_font_atlas();
}

#[cfg(feature = "legacy-monolith-proofs")]
fn build_font_atlas() {
    println!("cargo:rerun-if-changed=assets/InterDisplay-Regular.ttf");
    println!("cargo:rerun-if-changed=assets/InterDisplay-SemiBold.ttf");
    let regular = load("assets/InterDisplay-Regular.ttf");
    let semibold = load("assets/InterDisplay-SemiBold.ttf");
    let mut source = String::from(
        "#[derive(Clone, Copy)]\npub struct Glyph { pub bitmap: &'static [u8], pub width: u8, pub height: u8, pub xmin: i8, pub ymin: i8, pub advance: u8, pub px: u8 }\n",
    );
    bake(&mut source, &regular, "B", "body", 15.0);
    bake(&mut source, &semibold, "H", "heading", 19.0);
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR missing"));
    fs::write(out.join("inter_atlas.rs"), source).expect("failed to write Inter atlas");
}

#[cfg(feature = "legacy-monolith-proofs")]
fn load(path: &str) -> Font {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
    Font::from_bytes(bytes, FontSettings::default()).expect("invalid Inter font")
}

#[cfg(feature = "legacy-monolith-proofs")]
fn bake(source: &mut String, font: &Font, prefix: &str, function: &str, px: f32) {
    for code in (32u32..=126).chain(0x0400u32..=0x04ff) {
        let ch = char::from_u32(code).unwrap();
        let (metrics, bitmap) = font.rasterize(ch, px);
        writeln!(
            source,
            "static {prefix}{code}: [u8; {}] = {:?};",
            bitmap.len(),
            bitmap
        )
        .unwrap();
        writeln!(
            source,
            "const {prefix}M{code}: Glyph = Glyph {{ bitmap: &{prefix}{code}, width: {}, height: {}, xmin: {}, ymin: {}, advance: {}, px: {} }};",
            metrics.width.min(255),
            metrics.height.min(255),
            metrics.xmin.clamp(-128, 127),
            metrics.ymin.clamp(-128, 127),
            metrics.advance_width.round().clamp(1.0, 255.0) as u8,
            px as u8
        )
        .unwrap();
    }
    writeln!(
        source,
        "pub fn {function}(ch: char) -> Glyph {{ match ch as u32 {{"
    )
    .unwrap();
    for code in (32u32..=126).chain(0x0400u32..=0x04ff) {
        writeln!(source, "{code} => {prefix}M{code},").unwrap();
    }
    writeln!(source, "_ => {prefix}M63, }} }}").unwrap();
}
