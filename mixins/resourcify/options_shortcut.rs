// Ctrl+A, C, V and X in the search box.
fn resourcify_options_shortcut(
    game: &mut crate::game::Game<'_>,
    shortcut: horizon_client::clipboard::EditShortcut,
) -> ::core::ops::ControlFlow<bool> {
    if !crate::game::native_resourcify::is_open() {
        return ::core::ops::ControlFlow::Continue(());
    }
    ::core::ops::ControlFlow::Break(crate::game::native_resourcify_ui::shortcut(game, shortcut))
}
