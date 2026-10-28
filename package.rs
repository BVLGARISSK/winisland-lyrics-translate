use winisland_plugin_api::packager::PluginPackager;

fn main() {
    PluginPackager::from_cargo()
        .expect("read Cargo.toml")
        .include_dir("worker")
        .build()
        .expect("build plugin ZIP");
}
