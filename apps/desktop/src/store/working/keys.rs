//! Small keys for the column's pages that show one row of a list by its text id (an
//! environment, a background job): a [`Page`](crate::store::Page) is a plain value the column
//! copies around, so it carries the key and the store keeps the id. An id asked for twice has
//! one key.

use std::marker::PhantomData;

/// A key for an id of kind `T`.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key<T> {
    index: u32,
    kind: PhantomData<fn() -> T>,
}

impl<T> Clone for Key<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Key<T> {}

impl<T> Key<T> {
    fn at(index: u32) -> Self {
        Self {
            index,
            kind: PhantomData,
        }
    }
}

/// Every id asked for, by key.
#[derive(Debug)]
pub(crate) struct Keys<T>(Vec<T>);

impl<T> Default for Keys<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T: PartialEq> Keys<T> {
    /// The key of `id`: its own when it was asked for before.
    pub(crate) fn key(&mut self, id: T) -> Key<T> {
        let index = self
            .0
            .iter()
            .position(|known| *known == id)
            .unwrap_or_else(|| {
                self.0.push(id);
                self.0.len() - 1
            });
        Key::at(u32::try_from(index).unwrap_or(u32::MAX))
    }

    /// The id of `key`.
    #[must_use]
    pub(crate) fn get(&self, key: Key<T>) -> Option<&T> {
        self.0.get(usize::try_from(key.index).ok()?)
    }

    /// Every id asked for, with its key.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (Key<T>, &T)> {
        self.0
            .iter()
            .enumerate()
            .map(|(index, id)| (Key::at(u32::try_from(index).unwrap_or(u32::MAX)), id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_keeps_its_key() {
        let mut keys = Keys::default();
        let one = keys.key("runner".to_owned());
        let two = keys.key("browser".to_owned());
        assert_ne!(one, two);
        assert_eq!(keys.key("runner".to_owned()), one);
        assert_eq!(keys.get(two).map(String::as_str), Some("browser"));
        assert_eq!(keys.get(Key::at(9)), None);
        let all: Vec<(Key<String>, &str)> = keys.iter().map(|(k, id)| (k, id.as_str())).collect();
        assert_eq!(all, [(one, "runner"), (two, "browser")]);
    }
}
