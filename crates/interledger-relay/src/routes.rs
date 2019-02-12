/// A simple static routing table.
#[derive(Clone, Debug, PartialEq)]
pub struct RoutingTable<T>(Vec<Route<T>>);

// TODO peer.* route

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

#[derive(Clone, Debug, PartialEq)]
pub struct Route<T> {
    target_prefix: Vec<u8>, // TODO validate? "^[a-zA-Z0-9._~-]+$"
    next_hop: T,
}

//#[derive(Clone, Debug)]
//struct Address(Vec<u8>);

impl<T> Route<T> {
    #[inline]
    pub fn new(
        target_prefix: Vec<u8>,
        next_hop: T,
    ) -> Self {
        Route { target_prefix, next_hop }
    }

    #[inline]
    pub fn next_hop(&self) -> &T {
        &self.next_hop
    }
}

#[cfg(test)]
mod routing_table_tests {
    use super::*;

    #[test]
    fn test_resolve() {
        let table = RoutingTable::new(vec![
            Route::new(b"test.one".to_vec(), 123),
            Route::new(b"test.two".to_vec(), 456),
            Route::new(b"test.".to_vec(), 789),
        ]);
        let routes = &table.0;
        assert_eq!(table.resolve(b"test.one.alice"), Some(&routes[0]));
        assert_eq!(table.resolve(b"test.two__"), Some(&routes[1]));
        assert_eq!(table.resolve(b"test.three"), Some(&routes[2]));
        assert_eq!(table.resolve(b"example.test.one"), None);
    }

    #[test]
    fn test_resolve_catchall() {
        let table = RoutingTable::new(vec![
            Route::new(b"test.one".to_vec(), 123),
            Route::new(b"test.two".to_vec(), 456),
            Route::new(b"".to_vec(), 789),
        ]);
        assert_eq!(table.resolve(b"example.test.one"), Some(&table.0[2]));
    }
}
