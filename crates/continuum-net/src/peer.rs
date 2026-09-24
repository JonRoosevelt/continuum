use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use continuum_core::DeviceId;

use crate::error::NetError;
use crate::identity::{device_id_from_public, Identity};
use crate::noise::{self, Session};

pub struct Peer {
    stream: TcpStream,
    session: Session,
    public_key: Vec<u8>,
}

impl Peer {
    pub fn connect<A: ToSocketAddrs>(
        address: A,
        identity: &Identity,
        expected_public: &[u8],
    ) -> Result<Self, NetError> {
        let mut stream = TcpStream::connect(address)?;
        stream.set_nodelay(true)?;
        let session = noise::initiate(&mut stream, identity, expected_public)?;
        Self::from_session(stream, session)
    }

    pub fn accept(
        mut stream: TcpStream,
        identity: &Identity,
        mut is_allowed: impl FnMut(&[u8]) -> bool,
    ) -> Result<Self, NetError> {
        stream.set_nodelay(true)?;
        let session = noise::respond(&mut stream, identity)?;
        let peer = Self::from_session(stream, session)?;
        if !is_allowed(peer.public_key()) {
            return Err(NetError::UnknownPeer);
        }
        Ok(peer)
    }

    fn from_session(stream: TcpStream, session: Session) -> Result<Self, NetError> {
        let public_key = session
            .remote_static()
            .map(<[u8]>::to_vec)
            .ok_or(NetError::UnknownPeer)?;
        Ok(Self {
            stream,
            session,
            public_key,
        })
    }

    pub fn send(&mut self, payload: &[u8]) -> Result<(), NetError> {
        self.session.send(&mut self.stream, payload)
    }

    pub fn recv(&mut self) -> Result<Vec<u8>, NetError> {
        self.session.recv(&mut self.stream)
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), NetError> {
        self.stream.set_read_timeout(timeout)?;
        Ok(())
    }

    #[must_use]
    pub fn public_key(&self) -> &[u8] {
        &self.public_key
    }

    #[must_use]
    pub fn device_id(&self) -> DeviceId {
        device_id_from_public(&self.public_key)
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn pinned_handshake_between_peers() {
        let responder_id = Identity::generate().unwrap();
        let initiator_id = Identity::generate().unwrap();
        let responder_public = responder_id.public_key().to_vec();
        let initiator_public = initiator_id.public_key().to_vec();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let expected = initiator_public.clone();

        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut peer = Peer::accept(stream, &responder_id, |key| key == expected.as_slice())
                .expect("peer should be allowed");
            let message = peer.recv().unwrap();
            peer.send(&message).unwrap();
        });

        let mut peer = Peer::connect(address, &initiator_id, &responder_public).unwrap();
        assert_eq!(peer.device_id(), device_id_from_public(&responder_public));
        peer.send(b"clip").unwrap();
        assert_eq!(peer.recv().unwrap(), b"clip");

        handle.join().unwrap();
    }

    #[test]
    fn rejects_unknown_peer() {
        let responder_id = Identity::generate().unwrap();
        let initiator_id = Identity::generate().unwrap();
        let responder_public = responder_id.public_key().to_vec();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            Peer::accept(stream, &responder_id, |_| false).is_err()
        });

        let _ = Peer::connect(address, &initiator_id, &responder_public);
        assert!(handle.join().unwrap());
    }
}
