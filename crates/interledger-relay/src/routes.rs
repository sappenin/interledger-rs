/// A simple static routing table.
#[derive(Clone, Debug, PartialEq)]
pub struct RoutingTable<T>(Vec<Route<T>>);

// TODO peer.* routes
// TODO validate target prefix? "^[a-zA-Z0-9._~-]+$"

impl<T> RoutingTable<T> {
    #[inline]
    pub fn new(routes: Vec<Route<T>>) -> Self {
        RoutingTable(routes)
    }

    pub fn resolve(&self, destination: &[u8]) -> Option<&Route<T>> {
        self.0
            .iter()
            .find(|route| {
                destination.starts_with(&route.target_prefix)
            })
    }
}

impl<T> Default for RoutingTable<T> {
    fn default() -> Self {
        RoutingTable::new(Vec::new())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Route<T> {
    target_prefix: Vec<u8>,
    next_hop: T,
}

/* TODO
pub enum NextHop {
    Static(Uri),
    Template(
}
*/

impl<T> Route<T> {
    #[inline]
    pub fn new(
        target_prefix: Vec<u8>,
        next_hop: T,
    ) -> Self {
        Route { target_prefix, next_hop }
    }

    #[inline]
    pub fn target_prefix(&self) -> &[u8] {
        &self.target_prefix[..]
    }

    #[inline]
    pub fn next_hop(&self) -> &T {
        &self.next_hop
    }
}

#[cfg(test)]
mod test_routing_table {
    use super::*;

    #[test]
    fn test_resolve() {
        let table = RoutingTable::new(vec![
            Route::new(b"test.one".to_vec(), 123),
            Route::new(b"test.two".to_vec(), 456),
            Route::new(b"test.".to_vec(), 789),
        ]);
        let routes = &table.0;
        // Exact match.
        assert_eq!(table.resolve(b"test.one"), Some(&routes[0]));
        // Prefix match.
        assert_eq!(table.resolve(b"test.one.alice"), Some(&routes[0]));
        assert_eq!(table.resolve(b"test.two.bob"), Some(&routes[1]));
        assert_eq!(table.resolve(b"test.three"), Some(&routes[2]));
        // Dot separator isn't necessary.
        assert_eq!(table.resolve(b"test.two__"), Some(&routes[1]));
        // No matching prefix.
        assert_eq!(table.resolve(b"example.test.one"), None);
        assert_eq!(table.resolve(b""), None);
    }

    #[test]
    fn test_resolve_catch_all() {
        let table = RoutingTable::new(vec![
            Route::new(b"test.one".to_vec(), 123),
            Route::new(b"test.two".to_vec(), 456),
            Route::new(b"".to_vec(), 789),
        ]);
        assert_eq!(table.resolve(b"example.test.one"), Some(&table.0[2]));
    }
}
