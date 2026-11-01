// Keys in the browser: typing searches, the arrows pick, Escape leaves.
fn resourcify_options_key(
    game: &mut crate::game::Game<'_>,
    key: KeyCode,
    text: Option<&str>,
) -> ::core::ops::ControlFlow<Result<()>> {
    if !crate::game::native_resourcify::is_open() {
        return ::core::ops::ControlFlow::Continue(());
    }
    ::core::ops::ControlFlow::Break(crate::game::native_resourcify_ui::key(game, key, text))
}
