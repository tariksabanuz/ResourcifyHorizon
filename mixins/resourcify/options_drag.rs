// Dragging does nothing in the browser (the Options screen behind it must not feel it).
fn resourcify_options_drag(game: &mut crate::game::Game<'_>) -> ::core::ops::ControlFlow<Result<()>> {
    if !crate::game::native_resourcify::is_open() {
        return ::core::ops::ControlFlow::Continue(());
    }
    game.dragging = false;
    ::core::ops::ControlFlow::Break(Ok(()))
}
