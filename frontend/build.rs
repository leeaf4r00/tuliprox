use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    path::Path,
};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by Cargo");
    let i18n_dir = Path::new(&manifest_dir).join("public/assets/i18n");
    let assets = ["index.json", "en.json", "pt-BR.json", "ru.json", "ar.json"];
    let mut hasher = DefaultHasher::new();

    for asset in assets {
        let path = i18n_dir.join(asset);
        println!("cargo:rerun-if-changed={}", path.display());
        asset.hash(&mut hasher);
        fs::read(path).expect("i18n asset must be readable").hash(&mut hasher);
    }

    println!("cargo:rustc-env=TULIPROX_I18N_ASSET_FINGERPRINT={:016x}", hasher.finish());
}
