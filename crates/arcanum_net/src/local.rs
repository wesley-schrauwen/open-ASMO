//! A host in this process: a [`SimApp`] behind a [`LoopbackHost`], and optionally a [`LanHost`]
//! for players on the LAN. Single player runs on one; so does the host of a LAN game.

use std::io;
use std::net::{SocketAddr, ToSocketAddrs};

use arcanum_rules::RulesData;
use arcanum_sim::{PlayerId, SimApp};

use crate::{HostTransport, LoopbackClient, LoopbackHost, LanHost};

pub struct LocalHost {
    pub sim: SimApp,
    transport: LoopbackHost,
    udp: Option<LanHost>,
}

impl LocalHost {
    pub fn new(rules: RulesData) -> Self {
        Self { sim: SimApp::new(rules), transport: LoopbackHost::new(), udp: None }
    }

    /// A client end for `player`, in this process.
    pub fn connect(&self, player: PlayerId) -> LoopbackClient {
        self.transport.connect(player)
    }

    /// Takes players on the LAN at `addr` as well (TCP and UDP on one port). Returns the address bound.
    pub fn listen(&mut self, addr: impl ToSocketAddrs) -> io::Result<SocketAddr> {
        let udp = LanHost::bind(addr)?;
        let bound = udp.local_addr();
        self.udp = Some(udp);
        Ok(bound)
    }

    /// The LAN address taken, if listening.
    pub fn listening(&self) -> Option<SocketAddr> {
        self.udp.as_ref().map(LanHost::local_addr)
    }

    /// Hands received intents to the sim, runs one frame, and sends what it wrote. A player in
    /// this process cannot also be played over the LAN: such intents are dropped. Deltas for players
    /// who are gone are dropped.
    pub fn update(&mut self) {
        let mut intents = self.transport.recv_intents();
        if let Some(udp) = &self.udp {
            intents.extend(udp.recv_intents().into_iter().filter(|(p, _)| !self.transport.has_player(*p)));
        }
        self.sim.receive(intents);
        self.sim.update();
        for (player, delta) in self.sim.take_deltas() {
            let _ = match &self.udp {
                Some(udp) if !self.transport.has_player(player) => udp.send_delta(player, delta),
                _ => self.transport.send_delta(player, delta),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use arcanum_rules::CharacterAction;
    use arcanum_sim::{CharacterRequest, LedgerDelta, PlayerIntent};

    use super::*;
    use crate::{ClientTransport, LanClient};

    #[test]
    fn a_lan_player_makes_a_character_on_the_local_host() {
        let mut host = LocalHost::new(RulesData::default());
        host.sim.use_manual_clock();
        let local = host.connect(PlayerId(1));
        let addr = host.listen("127.0.0.1:0").unwrap();
        let lan = LanClient::connect(addr, PlayerId(2)).unwrap();
        let impostor = LanClient::connect(addr, PlayerId(1)).unwrap();
        let name = |n: &str| PlayerIntent::Character { seq: 1, request: CharacterRequest::Action(CharacterAction::SetName(n.into())) };
        lan.send_intent(name("Remote")).unwrap();
        impostor.send_intent(name("Impostor")).unwrap();
        let start = Instant::now();
        let mut got = Vec::new();
        while got.is_empty() {
            assert!(start.elapsed() < Duration::from_secs(5), "no answer over the LAN");
            host.update();
            got.extend(lan.recv_deltas());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(matches!(&got[0], LedgerDelta::CharacterUpdate { player: PlayerId(2), creation, .. } if creation.sheet.name == "Remote"));
        assert!(local.recv_deltas().is_empty(), "nothing for the local player");
        assert!(impostor.recv_deltas().is_empty(), "the local player's id is not playable over the LAN");
    }
}
