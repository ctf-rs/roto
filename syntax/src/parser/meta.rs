use std::{
    borrow::Borrow,
    fmt::Display,
    ops::{Deref, DerefMut, Range},
};

#[derive(Clone, Debug, Default)]
pub struct Spans(Vec<Span>);

impl Spans {
    /// Add a span for `x` and return the metadata-wrapped value.
    pub fn add<T>(&mut self, span: Span, x: T) -> Meta<T> {
        let id = MetaId(self.0.len());
        self.0.push(span);
        Meta { node: x, id }
    }

    /// Get the span associated with an AST metadata identifier.
    pub fn get(&self, x: impl Into<MetaId>) -> Span {
        self.0[x.into().0]
    }

    /// Return the smallest span containing both metadata identifiers.
    pub fn merge(
        &mut self,
        x: impl Into<MetaId>,
        y: impl Into<MetaId>,
    ) -> Span {
        self.get(x).merge(self.get(y))
    }

    /// Return the number of stored spans.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Return whether no spans are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over metadata identifiers and their exact byte spans.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (MetaId, Span)> + '_ {
        self.0
            .iter()
            .copied()
            .enumerate()
            .map(|(id, span)| (MetaId(id), span))
    }
}

impl<T> From<&Meta<T>> for MetaId {
    fn from(value: &Meta<T>) -> Self {
        value.id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: usize,
    pub start: usize,
    pub end: usize,
}

impl Span {
    /// Return the smallest span containing `self` and `other`.
    pub fn merge(self, other: Self) -> Self {
        assert_eq!(self.file, other.file);
        Self {
            file: self.file,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn new(file: usize, value: Range<usize>) -> Self {
        Self {
            file,
            start: value.start,
            end: value.end,
        }
    }

    /// Return this span's exact byte range.
    #[must_use]
    pub fn range(self) -> Range<usize> {
        self.start..self.end
    }

    /// Return whether this source span is empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Given the source file, returns a range of character offsets
    pub fn character_range(&self, file: &str) -> Range<usize> {
        let start = file[..self.start].chars().count();
        let end = start + file[self.start..self.end].chars().count();
        start..end
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MetaId(pub usize);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Meta<T> {
    pub node: T,
    pub id: MetaId,
}

impl<T: AsRef<U>, U: ?Sized> AsRef<U> for Meta<T> {
    fn as_ref(&self) -> &U {
        self.node.as_ref()
    }
}

impl<T: Borrow<str>> Borrow<str> for Meta<T> {
    fn borrow(&self) -> &str {
        self.node.borrow()
    }
}

impl<T: Display> Display for Meta<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.node.fmt(f)
    }
}

impl<T> Deref for Meta<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.node
    }
}

impl<T> DerefMut for Meta<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.node
    }
}
