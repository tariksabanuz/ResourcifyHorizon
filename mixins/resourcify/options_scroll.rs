// The wheel in the browser: the list or the detail pane, whichever the pointer is over.
fn resourcify_options_scroll(game: &mut crate::game::Game<'_>, amount: f32) -> ::core::ops::ControlFlow<Result<()>> {
    if !crate::game::native_resourcify::is_open() {
        return ::core::ops::ControlFlow::Continue(());
    }
    ::core::ops::ControlFlow::Break(crate::game::native_resourcify_ui::scroll(game, amount))
}
