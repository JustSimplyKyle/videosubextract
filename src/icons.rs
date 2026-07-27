use cosmic::widget::icon;
use manganis::Asset;

/// Resolve a Manganis asset to a native bundle path and retain symbolic
/// recoloring even when Manganis adds a hash to the staged filename.
#[must_use]
pub fn symbolic(asset: Asset) -> icon::Handle {
    let path = dioxus_asset_resolver::asset_path(asset)
        .expect("Manganis icon asset is missing from the native bundle");
    icon::from_path(path).symbolic(true)
}
