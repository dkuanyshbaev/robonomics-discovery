///////////////////////////////////////////////////////////////////////////////
//
//  Copyright 2018-2022 Robonomics Network <research@robonomics.network>
//
//  Licensed under the Apache License, Version 2.0 (the "License");
//  you may not use this file except in compliance with the License.
//  You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
//
///////////////////////////////////////////////////////////////////////////////
//! Robonomics Network discovery service.

use async_std::task;
use clap::Parser;
use futures::prelude::*;
use libp2p::{
    development_transport,
    gossipsub::{
        Gossipsub, GossipsubConfigBuilder, GossipsubEvent, GossipsubMessage, MessageAuthenticity,
        MessageId,
    },
    identity,
    kad::{record::store::MemoryStore, Kademlia, KademliaEvent},
    mdns::{Mdns, MdnsConfig, MdnsEvent},
    swarm::{behaviour::toggle::Toggle, NetworkBehaviourEventProcess},
    Multiaddr, NetworkBehaviour, PeerId, Swarm,
};
use std::{
    collections::hash_map::DefaultHasher,
    error::Error,
    hash::{Hash, Hasher},
    str::FromStr,
    time::Duration,
};

// Cli args parser.
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(long)]
    disable_mdns: bool,

    #[clap(long)]
    disable_kad: bool,

    #[clap(long, default_value_t = 1000)]
    heartbeat_interval: u64,

    #[clap(long, default_value_t = ("").to_string())]
    bootnodes: String,
}

// General behaviour of the network. Combines all protocols together.
#[derive(NetworkBehaviour)]
#[behaviour(event_process = true)]
struct RobonomicsNetworkBehaviour {
    pubsub: Gossipsub,
    mdns: Toggle<Mdns>,
    kademlia: Toggle<Kademlia<MemoryStore>>,
}

impl NetworkBehaviourEventProcess<MdnsEvent> for RobonomicsNetworkBehaviour {
    // Called when `mdns` produces an event.
    fn inject_event(&mut self, event: MdnsEvent) {
        match event {
            // Discovered nodes through mDNS.
            MdnsEvent::Discovered(list) => {
                for (peer_id, multiaddr) in list {
                    // Remove Ws component from multiaddr.
                    // let mut no_ws_multiaddr = Multiaddr::empty();
                    // for comp in multiaddr.iter() {
                    //     match comp {
                    //         Protocol::Ws(_) => continue,
                    //         _ => no_ws_multiaddr.push(comp),
                    //     };
                    // }
                    // log::info!("no_ws_multiaddr: {:?}", no_ws_multiaddr);

                    // self.pubsub.add_explicit_peer(&peer_id);

                    // let a = PeerId::try_from_multiaddr(peer).expect("multiaddr!");
                    // self.pubsub.add_explicit_peer(a);

                    // Add multiaddr to kad.
                    if let Some(kad) = self.kademlia.as_mut() {
                        // kad.add_address(&peer_id, no_ws_multiaddr);
                        kad.add_address(&peer_id, multiaddr);
                    };
                    log::info!("mDns discovered: {:?}", peer_id);
                }
            }
            // The given combinations of PeerId and Multiaddr have expired.
            MdnsEvent::Expired(list) => {
                for (peer_id, multiaddr) in list {
                    if let Some(kad) = self.kademlia.as_mut() {
                        kad.remove_address(&peer_id, &multiaddr);
                    };
                    log::info!("mDns expired: {:?}", peer_id);
                }
            }
        }
    }
}

impl NetworkBehaviourEventProcess<KademliaEvent> for RobonomicsNetworkBehaviour {
    // Called when `kademlia` produces an event.
    fn inject_event(&mut self, event: KademliaEvent) {
        match event {
            // The routing table has been updated with a new peer/address
            KademliaEvent::RoutingUpdated { peer, .. } => {
                log::info!("Kad new peer: {:?}", peer);
                // let a = self.kademlia.as_mut().expect("").bootstrap();
                // log::info!("a: {:?}", a);
                // -----------------------------------------------
                // for a in kademlia.kbuckets() {
                //     for i in a.iter() {
                //         log::info!(
                //             "))))))) {:?}, {:?}, {:?}",
                //             i.status,
                //             i.node.key,
                //             i.node.value
                //         );
                //     }
                // }
                // -----------------------------------------------
                // self.pubsub.add_explicit_peer(&peer);
                // let ps = self.pubsub.all_peers();
                // for p in ps {
                //     log::info!("---- pubsub peer: {:?}", p);
                // }
            }
            KademliaEvent::InboundRequest { request } => {
                log::info!("InboundRequest! {:?}", request);
            }
            KademliaEvent::PendingRoutablePeer { peer, address } => {
                log::info!("PendingRoutablePeer! {:?}, {:?}", peer, address);
            }
            KademliaEvent::RoutablePeer { peer, address } => {
                log::info!("RoutablePeer! {:?}, {:?}", peer, address);
            }
            KademliaEvent::UnroutablePeer { peer } => {
                log::info!("UnroutablePeer! {:?}", peer);
            }
            KademliaEvent::OutboundQueryCompleted { result, .. } => {
                log::info!("OutboundQueryCompleted! {:?}", result);
            }
        }
    }
}

impl NetworkBehaviourEventProcess<GossipsubEvent> for RobonomicsNetworkBehaviour {
    // Called when `gossipsub` produces an event.
    fn inject_event(&mut self, event: GossipsubEvent) {
        log::info!("Gossipsub event: {:?}", event);
        // if let GossipsubEvent::Message { message, .. } = event {
        //     log::info!("message: {:?}", message);
        // }
    }
}

#[async_std::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    let args = Args::parse();

    // Create a random PeerId.
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    log::info!("Local peer ID: {:?}", local_peer_id);

    // Set up a an encrypted DNS-enabled TCP Transport.
    let transport = development_transport(local_key.clone()).await?;

    let gossipsub_config = GossipsubConfigBuilder::default()
        .heartbeat_interval(Duration::from_millis(args.heartbeat_interval))
        .message_id_fn(|message: &GossipsubMessage| {
            // To content-address message,
            // we can take the hash of message and use it as an ID.
            let mut s = DefaultHasher::new();
            message.data.hash(&mut s);
            MessageId::from(s.finish().to_string())
        })
        .build()
        .expect("Valid gossipsub config");

    // Create PubSub
    let pubsub = Gossipsub::new(MessageAuthenticity::Signed(local_key), gossipsub_config)
        .expect("Correct configuration");

    // Use mDNS.
    let mdns = if !args.disable_mdns {
        log::info!("Using mDNS discovery service.");
        let mdns = task::block_on(Mdns::new(MdnsConfig::default()))?;
        Toggle::from(Some(mdns))
    } else {
        Toggle::from(None)
    };

    // Parse bootnodes.
    let bootnodes: Vec<&str> = args.bootnodes.split(",").map(|s| s.trim()).collect();
    let mut boot_peers: Vec<(PeerId, Multiaddr)> = vec![];
    for node in bootnodes {
        if let Ok(mut bootaddress) = Multiaddr::from_str(node) {
            if let Some(peer_id) = PeerId::try_from_multiaddr(&bootaddress) {
                if let Some(_) = bootaddress.pop() {
                    boot_peers.push((peer_id, bootaddress));
                }
            }
        }
    }

    // Use DHT.
    let kademlia = if !args.disable_kad {
        log::info!("Using DHT discovery service.");
        let store = MemoryStore::new(local_peer_id);
        let mut kademlia = Kademlia::new(local_peer_id, store);
        // Adding robonomics bootnodes.
        for (peer, bootaddr) in boot_peers {
            log::info!("Adding bootnode: {:?}", peer);
            kademlia.add_address(&peer, bootaddr);
        }
        Toggle::from(Some(kademlia))
    } else {
        Toggle::from(None)
    };

    // Custom NetworkBehaviour.
    let behaviour = RobonomicsNetworkBehaviour {
        // TODO: add reqresp
        kademlia,
        mdns,
        pubsub,
    };

    // Create a swarm to manage peers and events.
    let mut swarm = Swarm::new(transport, behaviour, local_peer_id);

    // Listen on all interfaces and whatever port the OS assigns.
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    loop {
        match swarm.select_next_some().await {
            // // ???
            // SwarmEvent::ConnectionEstablished { peer_id, .. } => {
            //     println!("Connected to: {:?}", peer_id);
            // }
            // // ???
            // SwarmEvent::Behaviour(event) => {
            //     println!("Event: {:?}", event);
            // }
            // SwarmEvent::NewListenAddr { address, .. } => {
            //     println!("Listening on {:?}", address);
            // }
            event => {
                println!(">>>> {:?}", event);
            } // _ => {}
        }
    }
}
