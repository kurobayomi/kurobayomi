//! Modal navigation stack of the tracking dialogs (`TrackNav`). The GUI maps each
//! page to a modal window; this decides which page is on top. It is generic so the pages can hold GUI-free models.

/// A page that is the Home page of the stack.
pub trait IsHome {
    fn is_home(&self) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackNav<P> {
    stack: Vec<P>,
}

impl<P> Default for TrackNav<P> {
    fn default() -> Self {
        Self { stack: Vec::new() }
    }
}

impl<P> TrackNav<P> {
    pub fn push(&mut self, page: P) {
        self.stack.push(page);
    }

    /// Removes the top page and returns it. `None` when the stack is empty (the sheet is closed).
    pub fn pop(&mut self) -> Option<P> {
        self.stack.pop()
    }

    pub fn top(&self) -> Option<&P> {
        self.stack.last()
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }
}

impl<P: IsHome> TrackNav<P> {
    /// `navigator.popUntil { it is HomeScreen }` (behaviour 8.4 and 9). Pops until a Home page is on top. Like
    /// Kotlin's `popUntil`, the stack empties when no Home page is on it.
    pub fn pop_until_home(&mut self) {
        while let Some(top) = self.stack.last() {
            if top.is_home() {
                break;
            }
            self.stack.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    enum Page {
        Home,
        Status,
        Date,
    }

    impl IsHome for Page {
        fn is_home(&self) -> bool {
            matches!(self, Page::Home)
        }
    }

    #[test]
    fn tr_27b_018_push_and_pop_follow_the_stack_order() {
        let mut nav = TrackNav::default();
        nav.push(Page::Home);
        nav.push(Page::Status);
        nav.push(Page::Date);
        assert_eq!(nav.len(), 3);
        assert_eq!(nav.pop(), Some(Page::Date));
        assert_eq!(nav.top(), Some(&Page::Status));
        assert!(!nav.is_empty());
    }

    #[test]
    fn tr_27b_018_pop_until_home_keeps_the_home_page_and_drops_the_rest() {
        let mut nav = TrackNav::default();
        nav.push(Page::Home);
        nav.push(Page::Status);
        nav.push(Page::Date);
        nav.pop_until_home();
        assert_eq!(nav.len(), 1);
        assert_eq!(nav.top(), Some(&Page::Home));
    }

    #[test]
    fn tr_27b_018_pop_until_home_empties_the_stack_when_no_home_is_on_it() {
        let mut nav = TrackNav::default();
        nav.push(Page::Status);
        nav.push(Page::Date);
        nav.pop_until_home();
        assert!(nav.is_empty());
    }

    #[test]
    fn tr_27b_018_popping_an_empty_stack_is_none() {
        let mut nav: TrackNav<Page> = TrackNav::default();
        assert_eq!(nav.pop(), None);
    }
}
