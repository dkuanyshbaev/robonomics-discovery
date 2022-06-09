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

use async_std::{io, task};
use clap::Parser;
use futures::{prelude::*, select};
use libp2p::{
    development_transport,
    gossipsub::{
        Gossipsub, GossipsubConfigBuilder, GossipsubEvent, GossipsubMessage, MessageAuthenticity,
        MessageId,
    },
    identity,
    kad::{
        record::{store::MemoryStore, Key},
        AddProviderOk, Kademlia, KademliaEvent, PeerRecord, PutRecordOk, QueryResult, Quorum,
        Record,
    },
    mdns::{Mdns, MdnsConfig, MdnsEvent},
    swarm::{NetworkBehaviourEventProcess, SwarmEvent},
    NetworkBehaviour, PeerId, Swarm,
};
use std::{
    collections::hash_map::{DefaultHasher, HashMap},
    error::Error,
    hash::{Hash, Hasher},
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
}

// A custom network behaviour that combines Kademlia, mDNS, and Gossipsub.
#[derive(NetworkBehaviour)]
#[behaviour(event_process = true)]
struct MyBehaviour {
    kademlia: Kademlia<MemoryStore>,
    mdns: Mdns,
    pubsub: Gossipsub,
}

impl NetworkBehaviourEventProcess<MdnsEvent> for MyBehaviour {
    // Called when `mdns` produces an event.
    fn inject_event(&mut self, event: MdnsEvent) {
        if let MdnsEvent::Discovered(list) = event {
            for (peer_id, multiaddr) in list {
                log::info!(">>>> peer_id: {:?}, multiaddr: {:?}", peer_id, multiaddr);
                self.kademlia.add_address(&peer_id, multiaddr);
            }
        }
    }
}

impl NetworkBehaviourEventProcess<KademliaEvent> for MyBehaviour {
    // Called when `kademlia` produces an event.
    fn inject_event(&mut self, message: KademliaEvent) {
        match message {
            KademliaEvent::OutboundQueryCompleted { result, .. } => match result {
                QueryResult::GetProviders(Ok(ok)) => {
                    for peer in ok.providers {
                        println!(
                            "Peer {:?} provides key {:?}",
                            peer,
                            std::str::from_utf8(ok.key.as_ref()).unwrap()
                        );
                    }
                }
                // QueryResult::GetProviders(Err(err)) => {
                //     eprintln!("Failed to get providers: {:?}", err);
                // }
                // QueryResult::GetRecord(Ok(ok)) => {
                //     for PeerRecord {
                //         record: Record { key, value, .. },
                //         ..
                //     } in ok.records
                //     {
                //         println!(
                //             "Got record {:?} {:?}",
                //             std::str::from_utf8(key.as_ref()).unwrap(),
                //             std::str::from_utf8(&value).unwrap(),
                //         );
                //     }
                // }
                // QueryResult::GetRecord(Err(err)) => {
                //     eprintln!("Failed to get record: {:?}", err);
                // }
                // QueryResult::PutRecord(Ok(PutRecordOk { key })) => {
                //     println!(
                //         "Successfully put record {:?}",
                //         std::str::from_utf8(key.as_ref()).unwrap()
                //     );
                // }
                // QueryResult::PutRecord(Err(err)) => {
                //     eprintln!("Failed to put record: {:?}", err);
                // }
                // QueryResult::StartProviding(Ok(AddProviderOk { key })) => {
                //     println!(
                //         "Successfully put provider record {:?}",
                //         std::str::from_utf8(key.as_ref()).unwrap()
                //     );
                // }
                // QueryResult::StartProviding(Err(err)) => {
                //     eprintln!("Failed to put provider record: {:?}", err);
                // }
                _ => {}
            },
            _ => {}
        }
    }
}

impl NetworkBehaviourEventProcess<GossipsubEvent> for MyBehaviour {
    // Called when `gossipsub` produces an event.
    fn inject_event(&mut self, event: GossipsubEvent) {
        log::info!("Gossipsub event");
        // if let MdnsEvent::Discovered(list) = event {
        //     for (peer_id, multiaddr) in list {
        //         log::info!(">>>> peer_id: {:?}, multiaddr: {:?}", peer_id, multiaddr);
        //         self.kademlia.add_address(&peer_id, multiaddr);
        //     }
        // }
    }
}

#[async_std::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    // Get cli args.
    let args = Args::parse();

    // Create a random PeerId.
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    log::info!("Local peer ID: {:?}", local_peer_id);

    // Set up a an encrypted DNS-enabled TCP Transport over the Mplex protocol.
    let transport = development_transport(local_key.clone()).await?;

    // ???
    let heartbeat_interval = Duration::from_millis(1000);

    // Create a swarm to manage peers and events.
    let mut swarm = {
        let store = MemoryStore::new(local_peer_id);
        let kademlia = Kademlia::new(local_peer_id, store);
        let mdns = task::block_on(Mdns::new(MdnsConfig::default()))?;
        //-------------------------------------------------------------
        let gossipsub_config = GossipsubConfigBuilder::default()
            .heartbeat_interval(heartbeat_interval)
            .message_id_fn(|message: &GossipsubMessage| {
                // To content-address message,
                // we can take the hash of message and use it as an ID.
                let mut s = DefaultHasher::new();
                message.data.hash(&mut s);
                MessageId::from(s.finish().to_string())
            })
            .build()
            .expect("Valid gossipsub config");

        let pubsub = Gossipsub::new(MessageAuthenticity::Signed(local_key), gossipsub_config)
            .expect("Correct configuration");
        //-------------------------------------------------------------

        let behaviour = MyBehaviour {
            kademlia,
            mdns,
            pubsub,
        };
        Swarm::new(transport, behaviour, local_peer_id)
    };

    // Read full lines from stdin
    // let mut stdin = io::BufReader::new(io::stdin()).lines().fuse();

    // Listen on all interfaces and whatever port the OS assigns.
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    // Start DHT.
    // if !args.disable_kad {
    //     log::info!("Starting DHT discovery service.");
    //     protocol::dht::kad(local_key.clone(), local_peer_id.clone()).await?;
    // }

    // Start mDNS.
    // if !args.disable_mdns {
    //     log::info!("Starting MDNS discovery service.");
    //     protocol::mdns::mdns(local_key, local_peer_id).await?;
    // }

    loop {
        select! {
            // line = stdin.select_next_some() => handle_input_line(&mut swarm.behaviour_mut().kademlia, line.expect("Stdin not to close")),
            event = swarm.select_next_some() => match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("Listening in {:?}", address);
                },
                _ => {}
            }
        }
    }
}
