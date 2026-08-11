//! Query projected client-session lifecycle.

use crate::{
    query::{Evidence, Query, select},
    session::{Connection, Epoch},
};

/// Resolves once a session observation established an active connection.
pub fn active() -> impl Query<Output = Evidence<Connection>> {
    select(|snapshot| {
        (snapshot.session_epoch() > Epoch::INITIAL && snapshot.connection() == Connection::Active)
            .then(|| Evidence::new(Connection::Active, snapshot.revision()))
    })
}

/// Resolves once either protocol or transport evidence closes the session.
pub fn closed() -> impl Query<Output = Evidence<Connection>> {
    select(|snapshot| {
        let connection = snapshot.connection();

        (connection != Connection::Active).then(|| Evidence::new(connection, snapshot.revision()))
    })
}
