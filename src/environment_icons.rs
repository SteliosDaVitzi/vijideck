//! Stable, offline group identities; never infer branding from executable paths.
use eframe::egui;

pub(super) fn logo(group: &str) -> &'static [u8] {
    match group {
        "Python" => include_bytes!("../assets/dev-tools/python.svg"),
        "Node.js" => include_bytes!("../assets/dev-tools/nodejs.svg"),
        ".NET" => include_bytes!("../assets/dev-tools/dotnetcore.svg"),
        "Java" => include_bytes!("../assets/dev-tools/java.svg"),
        "Rust" => include_bytes!("../assets/dev-tools/rust.svg"),
        "Go" => include_bytes!("../assets/dev-tools/go.svg"),
        "Ruby" => include_bytes!("../assets/dev-tools/ruby.svg"),
        "PHP" => include_bytes!("../assets/dev-tools/php.svg"),
        "Deno" => include_bytes!("../assets/dev-tools/denojs.svg"),
        "Bun" => include_bytes!("../assets/dev-tools/bun.svg"),
        "Containers" => include_bytes!("../assets/dev-tools/docker.svg"),
        "Git" => include_bytes!("../assets/dev-tools/git.svg"),
        "Build tools" => include_bytes!("../assets/dev-tools/build.svg"),
        "System package managers" => include_bytes!("../assets/dev-tools/packages.svg"),
        _ => include_bytes!("../assets/dev-tools/tools.svg"),
    }
}

pub(super) const LOGO_SIZE: f32 = 30.0;

pub(super) fn pixel_size(ctx: &egui::Context) -> u32 {
    (LOGO_SIZE * ctx.pixels_per_point())
        .round()
        .clamp(1.0, 512.0) as u32
}

pub(super) fn load(
    ctx: &egui::Context,
    group: &str,
    pixels: u32,
) -> Result<egui::TextureHandle, String> {
    const SUPERSAMPLE: u32 = 4;
    let image = egui_extras::image::load_svg_bytes_with_size(
        logo(group),
        egui::load::SizeHint::Size {
            width: pixels * SUPERSAMPLE,
            height: pixels * SUPERSAMPLE,
            maintain_aspect_ratio: true,
        },
        &Default::default(),
    )?;
    // Resample premultiplied RGBA on the CPU so the GPU does not have to
    // minify a much larger texture without mipmaps.
    let rgba: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_array())
        .collect();
    let source = image::RgbaImage::from_raw(image.size[0] as u32, image.size[1] as u32, rgba)
        .ok_or_else(|| "Invalid SVG image dimensions".to_owned())?;
    let width = ((image.size[0] as u32 + SUPERSAMPLE / 2) / SUPERSAMPLE).max(1);
    let height = ((image.size[1] as u32 + SUPERSAMPLE / 2) / SUPERSAMPLE).max(1);
    let resized = image::imageops::resize(
        &source,
        width,
        height,
        image::imageops::FilterType::Lanczos3,
    );
    let image = egui::ColorImage::from_rgba_premultiplied(
        [width as usize, height as usize],
        resized.as_raw(),
    );
    Ok(ctx.load_texture(
        format!("dev-tool-logo-{group}-{pixels}"),
        image,
        egui::TextureOptions::LINEAR,
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_group_logos_render() {
        let ctx = eframe::egui::Context::default();
        for group in [
            "Python",
            "Node.js",
            ".NET",
            "Java",
            "Rust",
            "Go",
            "Ruby",
            "PHP",
            "Deno",
            "Bun",
            "Containers",
            "Git",
            "Build tools",
            "System package managers",
            "Other tools",
        ] {
            for pixels in [30, 45, 60] {
                let texture = super::load(&ctx, group, pixels)
                    .unwrap_or_else(|error| panic!("{group}: {error}"));
                assert!(
                    texture
                        .size()
                        .iter()
                        .all(|&dimension| dimension > 0 && dimension <= pixels as usize)
                );
            }
        }
    }
}
