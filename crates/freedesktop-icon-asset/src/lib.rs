use proc_macro::TokenStream;
use quote::quote;
use std::path::{Path, PathBuf};
use syn::{LitStr, parse_macro_input};

const ICON_THEME: &str = "Cosmic";
const GENERATED_ASSET_DIR: &str = "assets/freedesktop-icons";

/// Resolve a named COSMIC icon at compile time and register the staged SVG
/// with Manganis.
///
/// The resolved icon is copied into the calling package because Manganis
/// intentionally only accepts assets contained by that package.
#[proc_macro]
pub fn freedesktop_icon_asset(input: TokenStream) -> TokenStream {
    let name = parse_macro_input!(input as LitStr);

    match expand(&name) {
        Ok(tokens) => tokens,
        Err(error) => {
            let message = error.to_string();
            quote!(compile_error!(#message)).into()
        }
    }
}

fn expand(name: &LitStr) -> Result<TokenStream, Box<dyn std::error::Error>> {
    let name_value = name.value();
    validate_name(&name_value)?;

    let source = cosmic_freedesktop_icons::lookup(&name_value)
        .with_theme(ICON_THEME)
        .force_svg()
        .find()
        .ok_or_else(|| {
            format!("could not find Freedesktop icon `{name_value}` in theme `{ICON_THEME}`")
        })?;

    if source.extension().and_then(|extension| extension.to_str()) != Some("svg") {
        return Err(format!(
            "Freedesktop icon `{name_value}` resolved to non-SVG asset {}",
            source.display()
        )
        .into());
    }

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let relative_path = PathBuf::from(GENERATED_ASSET_DIR).join(format!("{name_value}.svg"));
    let destination = manifest_dir.join(&relative_path);
    stage_if_changed(&source, &destination)?;

    let asset_path = format!("/{}", relative_path.to_string_lossy().replace('\\', "/"));
    Ok(quote! {
        {
            const _: manganis::Asset = manganis::asset!(
                "/assets/freedesktop-icons/COPYING",
                manganis::AssetOptions::builder().with_hash_suffix(false)
            );
            #[allow(volatile_composites)]
            manganis::asset!(#asset_path)
        }
    }
    .into())
}

fn validate_name(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!("invalid Freedesktop icon name `{name}`").into());
    }

    Ok(())
}

fn stage_if_changed(source: &Path, destination: &Path) -> std::io::Result<()> {
    let source_bytes = std::fs::read(source)?;
    if matches!(
        std::fs::read(destination),
        Ok(existing) if existing == source_bytes
    ) {
        return Ok(());
    }

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(destination, source_bytes)
}
