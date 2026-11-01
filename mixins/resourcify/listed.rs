// Resourcify's row in Options > Mods. It has no keys and no settings.
fn resourcify_listed(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    use crate::game::native_resourcify as r;
    mods.push(crate::game::native_hooks::NativeMod {
        id: r::ID.into(),
        title: format!("{} {}", r::NAME, r::VERSION),
        description: r::DESCRIPTION.into(),
        keys: Vec::new(),
    });
}
