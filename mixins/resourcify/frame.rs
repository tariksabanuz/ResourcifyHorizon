// If the Options screen is gone without the browser having been left (a world loading, a
// disconnect), the next Options screen is the normal one.
fn resourcify_frame(game: &mut crate::game::Game<'_>) {
    if crate::game::native_resourcify::is_open() && game.screen != crate::game::Screen::Options {
        crate::game::native_resourcify::close();
    }
}
