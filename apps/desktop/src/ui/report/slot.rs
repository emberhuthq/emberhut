//! The panel's one web view: made when a report shows, kept while the same report shows, and
//! dropped when the panel closes or shows something else. Never two at once.

/// The view (or why there is none) for the report it was made for.
#[derive(Debug)]
struct Shown<V, E> {
    /// The report's path.
    key: String,
    view: Result<V, E>,
}

/// At most one view, for one report.
#[derive(Debug)]
pub(super) struct Slot<V, E> {
    shown: Option<Shown<V, E>>,
}

impl<V, E> Default for Slot<V, E> {
    fn default() -> Self {
        Self { shown: None }
    }
}

impl<V, E> Slot<V, E> {
    /// The view for the report at `key`, made with `make` unless it is the one already shown;
    /// the view shown before goes first. A failure is kept as well, so a report whose view
    /// can't be made isn't tried again on every frame.
    pub(super) fn show(
        &mut self,
        key: &str,
        make: impl FnOnce() -> Result<V, E>,
    ) -> Result<&V, &E> {
        let shown = match self.shown.take() {
            Some(shown) if shown.key == key => shown,
            old => {
                drop(old);
                Shown {
                    key: key.to_owned(),
                    view: make(),
                }
            }
        };
        self.shown.insert(shown).view.as_ref()
    }

    /// Drops the view, if any.
    pub(super) fn clear(&mut self) {
        self.shown = None;
    }

    /// The report the view is for.
    #[cfg(test)]
    pub(super) fn key(&self) -> Option<&str> {
        self.shown.as_ref().map(|shown| shown.key.as_str())
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::Slot;

    /// A view that counts how many of it are alive and how many were ever made.
    #[derive(Debug)]
    pub(in crate::ui::report) struct Probe {
        live: Rc<Cell<usize>>,
    }

    /// The counters a [`Probe`] keeps.
    #[derive(Clone, Debug, Default)]
    pub(in crate::ui::report) struct Probes {
        live: Rc<Cell<usize>>,
        made: Rc<Cell<usize>>,
    }

    impl Probes {
        pub(in crate::ui::report) fn make(&self) -> Probe {
            self.live.set(self.live.get() + 1);
            self.made.set(self.made.get() + 1);
            Probe {
                live: Rc::clone(&self.live),
            }
        }

        pub(in crate::ui::report) fn live(&self) -> usize {
            self.live.get()
        }

        pub(in crate::ui::report) fn made(&self) -> usize {
            self.made.get()
        }
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            self.live.set(self.live.get() - 1);
        }
    }

    #[test]
    fn the_same_report_keeps_its_view() {
        let probes = Probes::default();
        let mut slot = Slot::<Probe, String>::default();
        for _ in 0..3 {
            assert!(slot.show("/a.html", || Ok(probes.make())).is_ok());
        }
        assert_eq!((probes.made(), probes.live()), (1, 1));
    }

    #[test]
    fn another_report_replaces_the_view_never_two_at_once() {
        let probes = Probes::default();
        let mut slot = Slot::<Probe, String>::default();
        assert!(slot.show("/a.html", || Ok(probes.make())).is_ok());
        let result = slot.show("/b.html", || {
            assert_eq!(
                probes.live(),
                0,
                "the first view is gone before the next is made"
            );
            Ok(probes.make())
        });
        assert!(result.is_ok());
        assert_eq!((probes.made(), probes.live()), (2, 1));
        assert_eq!(slot.key(), Some("/b.html"));
    }

    #[test]
    fn clearing_drops_the_view() {
        let probes = Probes::default();
        let mut slot = Slot::<Probe, String>::default();
        assert!(slot.show("/a.html", || Ok(probes.make())).is_ok());
        slot.clear();
        assert_eq!((probes.live(), slot.key()), (0, None));
    }

    #[test]
    fn a_failure_is_kept_for_its_report_and_not_retried() {
        let probes = Probes::default();
        let mut slot = Slot::<Probe, String>::default();
        let tries = Cell::new(0);
        for _ in 0..3 {
            let result = slot.show("/a.html", || {
                tries.set(tries.get() + 1);
                Err("no web view".to_owned())
            });
            assert_eq!(result.err().map(String::as_str), Some("no web view"));
        }
        assert_eq!(tries.get(), 1);
        // Another report tries again.
        assert!(slot.show("/b.html", || Ok(probes.make())).is_ok());
        assert_eq!(probes.live(), 1);
    }
}
