//! Root navigator (Kotlin `Navigator`, Voyager). One stack per window; `Route::Home` is always the root.
//!
//! `Navigator` is pure.

#[cfg(test)]
use kuro_platform::kind::Kind;

#[allow(dead_code)] // kept for the shape of the type; not built or read yet
#[cfg(test)]
/// One page of the stack. Only `Home` is ever the root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Home,
    Entry { kind: Kind, id: i64, from_source: bool },
    GlobalSearch { kind: Kind, query: String },
    Settings { dest: Option<String> },
    Downloads,
    RestoreBackup { uri: String },
}

#[cfg(test)]
/// Direction of the last stack change. A pop slides back; everything else slides forward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Back,
}

#[cfg(test)]
/// The stack of routes. The invariant `stack[0] == Route::Home` holds for its whole life.
#[derive(Debug)]
pub struct Navigator {
    stack: Vec<Route>,
    last: Direction,
}

#[cfg(test)]
impl Default for Navigator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl Navigator {
    pub fn new() -> Self {
        Self { stack: vec![Route::Home], last: Direction::Forward }
    }

    pub fn stack(&self) -> &[Route] {
        &self.stack
    }

    /// The top route (Voyager `lastItem`).
    pub fn last(&self) -> &Route {
        self.stack.last().unwrap_or(&Route::Home)
    }

    #[cfg(test)]
    pub fn direction(&self) -> Direction {
        self.last
    }

    pub fn push(&mut self, route: Route) {
        self.stack.push(route);
        self.last = Direction::Forward;
    }

    /// Removes the top route. Returns `None` on the root, which is never removed.
    pub fn pop(&mut self) -> Option<Route> {
        if self.stack.len() <= 1 {
            return None;
        }
        self.last = Direction::Back;
        self.stack.pop()
    }

    /// Replaces the top route and returns the old one. A replace on the root is refused and returns `None`.
    pub fn replace(&mut self, route: Route) -> Option<Route> {
        if self.stack.len() <= 1 {
            return None;
        }
        self.last = Direction::Forward;
        self.stack.last_mut().map(|top| std::mem::replace(top, route))
    }

    #[cfg(test)]
    /// Pops everything above `Home` and returns the popped routes, top first.
    pub fn pop_until_root(&mut self) -> Vec<Route> {
        let mut popped = Vec::new();
        while let Some(route) = self.pop() {
            popped.push(route);
        }
        popped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i64) -> Route {
        Route::Entry { kind: Kind::Anime, id, from_source: false }
    }

    #[test]
    fn root_is_home_and_never_popped() {
        let mut nav = Navigator::new();
        assert_eq!(nav.stack(), &[Route::Home]);
        assert_eq!(nav.pop(), None);
        assert_eq!(nav.stack(), &[Route::Home]);
    }

    #[test]
    fn push_then_pop_restores_the_previous_top() {
        let mut nav = Navigator::new();
        nav.push(entry(1));
        nav.push(Route::Downloads);
        assert_eq!(nav.last(), &Route::Downloads);
        assert_eq!(nav.pop(), Some(Route::Downloads));
        assert_eq!(nav.last(), &entry(1));
    }

    #[test]
    fn replace_swaps_the_top_and_refuses_the_root() {
        let mut nav = Navigator::new();
        assert_eq!(nav.replace(Route::Downloads), None);
        assert_eq!(nav.stack(), &[Route::Home]);
        nav.push(entry(1));
        assert_eq!(nav.replace(entry(2)), Some(entry(1)));
        assert_eq!(nav.stack(), &[Route::Home, entry(2)]);
    }

    #[test]
    fn pop_until_root_returns_the_popped_pages_top_first() {
        let mut nav = Navigator::new();
        nav.push(entry(1));
        nav.push(entry(2));
        nav.push(Route::Settings { dest: None });
        let popped = nav.pop_until_root();
        assert_eq!(popped, vec![Route::Settings { dest: None }, entry(2), entry(1)]);
        assert_eq!(nav.stack(), &[Route::Home]);
    }

    #[test]
    fn direction_follows_the_last_change() {
        let mut nav = Navigator::new();
        nav.push(entry(1));
        assert_eq!(nav.direction(), Direction::Forward);
        nav.pop();
        assert_eq!(nav.direction(), Direction::Back);
        nav.pop();
        assert_eq!(nav.direction(), Direction::Back, "a refused pop keeps the last direction");
    }
}
