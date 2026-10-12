//! Podman networks.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{
    store::Store,
    teardown::{OnDrop, Teardown},
    WeakContainer,
};
use crate::error::Error;

/// The label key that records a network owner.
const OWNER_LABEL: &str = "id2202.owner";

/// A selection of JSON output fields when listing the networks
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NetworkInfo {
    pub name: String,
    pub id: String,
    pub driver: String,
    pub network_interface: String,
    pub created: String,
    pub ipv6_enabled: bool,
    pub internal: bool,
    pub dns_enabled: bool,
}

/// The podman networks in `store`.
pub fn networks(store: &Store) -> Result<Vec<NetworkInfo>, Error> {
    store.podman_cmd_json(["network", "list", "--format", "json"])
}

/// The names of podman network in `store`.
pub fn network_names(store: &Store) -> Result<Vec<String>, Error> {
    let netlist = networks(store)?;
    let nets: BTreeSet<String> = BTreeSet::from_iter(netlist.into_iter().map(|pno| pno.name));
    Ok(Vec::from_iter(nets))
}

/// Removes unused networks in `store` that was made for `owner` using
/// [`WeakNetwork::create`].
pub fn prune_networks(store: &Store, owner: &str) -> Result<(), Error> {
    store.podman_cmd([
        "network",
        "prune",
        "--force",
        "--filter",
        &format!("label={OWNER_LABEL}={owner}"),
    ])?;
    Ok(())
}

/// Removes unused networks that was created using [`WeakNetwork::create`].
pub fn prune_owned_networks(store: &Store) -> Result<(), Error> {
    store.podman_cmd([
        "network",
        "prune",
        "--force",
        "--filter",
        &format!("label={OWNER_LABEL}"),
    ])?;
    Ok(())
}

/// A strong reference to a network, which will get removed when dropped.
pub type Network = OnDrop<WeakNetwork>;

impl Network {
    /// Wrapper for [`WeakNetwork::create`].
    pub fn create(store: Store, name: impl Into<String>, owner: &str) -> Result<Network, Error> {
        WeakNetwork::create(store, name, owner).map(Network::from)
    }
}

/// A network that containers attach to.
///
/// # Note
/// The network is left as is when this struct is dropped.
/// (Weak resource reference.)
#[derive(Debug)]
pub struct WeakNetwork {
    name: String,
    store: Store,
}

impl WeakNetwork {
    /// Creates a network in `store` named `name`, recording its `owner`. The
    /// owner can be used later when pruning the network.
    ///
    /// # Warning
    /// If a network with the same name already exists, then it will be removed
    /// by this function.
    pub fn create(
        store: Store,
        name: impl Into<String>,
        owner: &str,
    ) -> Result<WeakNetwork, Error> {
        let network = WeakNetwork { name: name.into(), store };

        // TODO: Warn about
        network.store.podman_cmd(["network", "rm", "--force", network.name.as_str()])?;

        network.store.podman_cmd([
            "network",
            "create",
            "--disable-dns",
            "--label",
            &format!("{OWNER_LABEL}={owner}"),
            network.name.as_str(),
        ])?;

        Ok(network)
    }

    /// The name containers attach to this network by.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The storage this network's commands run in. Containers attaching to it
    /// should use the same storage.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Disconnects the container from the network.
    pub fn disconnect(&self, container: &WeakContainer) -> Result<(), Error> {
        self.store.podman_cmd(["network", "disconnect", &self.name, container.name()])?;
        Ok(())
    }
}

impl Teardown for WeakNetwork {
    /// Force removes the podman network.
    fn teardown(&mut self) -> Result<(), Error> {
        self.store
            .podman_cmd(["network", "rm", "--force", self.name.as_str()])
            .map_err(|e| {
                Error::runtime(format!(
                    "could not teardown the underlying network \"{}\"",
                    self.name
                ))
                .with_cause(e)
            })
            .map(|_| ())
    }
}
