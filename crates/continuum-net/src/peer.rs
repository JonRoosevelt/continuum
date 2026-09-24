use std::io::{ErrorKind, Read as _};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use continuum_core::DeviceId;

use crate::error::NetError;
use crate::identity::{device_id_from_public, Identity};
use crate::noise::{self, write_frame, Session};
use crate::protocol::Message;

const READ_TIMEOUT: Duration = Duration::from_millis(50);
const READ_CHUNK: usize = 64 * 1024;
const MAX_FRAME: u32 = 64 * 1024 * 1024;
const PING_INTERVAL: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
// Noise caps a single message at 65535 bytes including the 16-byte tag; chunk below that.
const NOISE_PLAIN_MAX: usize = 60_000;

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

    /// Serves a peer connection until it disconnects: flushes queued outbound frames,
    /// answers pings, and dispatches decoded inbound messages. A short read timeout lets a
    /// single thread both send and receive; a keepalive detects dead peers.
    pub fn run(
        &mut self,
        outbound: Receiver<Vec<u8>>,
        mut on_message: impl FnMut(Message),
    ) -> Result<(), NetError> {
        self.stream.set_read_timeout(Some(READ_TIMEOUT))?;
        let mut buffer: Vec<u8> = Vec::new();
        let mut reassembly: Vec<u8> = Vec::new();
        let mut chunk = vec![0u8; READ_CHUNK];
        let mut last_ping = Instant::now();
        let mut last_seen = Instant::now();

        loop {
            loop {
                match outbound.try_recv() {
                    Ok(payload) => self.write_message(&payload)?,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Ok(()),
                }
            }

            if last_ping.elapsed() >= PING_INTERVAL {
                self.write_message(&Message::Ping.encode()?)?;
                last_ping = Instant::now();
            }

            match self.stream.read(&mut chunk) {
                Ok(0) => return Ok(()),
                Ok(n) => {
                    buffer.extend_from_slice(&chunk[..n]);
                    last_seen = Instant::now();
                }
                Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(err) => return Err(NetError::Io(err)),
            }

            while let Some(frame) = take_frame(&mut buffer)? {
                reassembly.extend_from_slice(&self.session.open(&frame)?);
                while reassembly.len() >= 4 {
                    let len = u32::from_be_bytes([
                        reassembly[0],
                        reassembly[1],
                        reassembly[2],
                        reassembly[3],
                    ]) as usize;
                    if len > MAX_FRAME as usize {
                        return Err(NetError::FrameTooLarge(len as u32));
                    }
                    if reassembly.len() < 4 + len {
                        break;
                    }
                    let message = reassembly[4..4 + len].to_vec();
                    reassembly.drain(..4 + len);
                    match Message::decode(&message)? {
                        Message::Ping => self.write_message(&Message::Pong.encode()?)?,
                        Message::Pong => {}
                        other => on_message(other),
                    }
                }
            }

            if last_seen.elapsed() >= IDLE_TIMEOUT {
                return Err(NetError::TimedOut);
            }
        }
    }

    /// Sends one logical message, chunked so no single Noise message exceeds the 64 KiB cap.
    fn write_message(&mut self, payload: &[u8]) -> Result<(), NetError> {
        let mut stream = Vec::with_capacity(4 + payload.len());
        let len = u32::try_from(payload.len()).map_err(|_| NetError::FrameTooLarge(u32::MAX))?;
        stream.extend_from_slice(&len.to_be_bytes());
        stream.extend_from_slice(payload);
        for chunk in stream.chunks(NOISE_PLAIN_MAX) {
            let sealed = self.session.seal(chunk)?;
            write_frame(&mut self.stream, &sealed)?;
        }
        Ok(())
    }
}

fn take_frame(buffer: &mut Vec<u8>) -> Result<Option<Vec<u8>>, NetError> {
    if buffer.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
    if len > MAX_FRAME {
        return Err(NetError::FrameTooLarge(len));
    }
    let total = 4 + len as usize;
    if buffer.len() < total {
        return Ok(None);
    }
    let frame = buffer[4..total].to_vec();
    buffer.drain(..total);
    Ok(Some(frame))
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

    #[test]
    fn transfers_large_payload_across_noise_chunks() {
        use continuum_core::{ClipItem, ClipboardItem, Representation, Version, PLAIN_TEXT_MIME};

        let responder_id = Identity::generate().unwrap();
        let initiator_id = Identity::generate().unwrap();
        let responder_public = responder_id.public_key().to_vec();
        let initiator_public = initiator_id.public_key().to_vec();

        let big = "a".repeat(200_000);
        let device = initiator_id.device_id();
        let item = ClipItem::new(vec![Representation::text(PLAIN_TEXT_MIME, &big)]).unwrap();
        let payload = ClipboardItem::new(device, Version::new(1, device), vec![item]).unwrap();
        let encoded = Message::Clipboard(Box::new(payload)).encode().unwrap();
        let expected = big.len();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let allowed = initiator_public;

        let responder = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut peer = Peer::accept(stream, &responder_id, |key| key == allowed.as_slice())
                .expect("peer allowed");
            let (out_tx, out_rx) = std::sync::mpsc::channel::<Vec<u8>>();
            let (result_tx, result_rx) = std::sync::mpsc::channel::<usize>();
            let reader = std::thread::spawn(move || {
                peer.run(out_rx, |message| {
                    if let Message::Clipboard(item) = message {
                        if let Some(text) = item.plain_text() {
                            let _ = result_tx.send(text.len());
                        }
                    }
                })
                .unwrap();
            });
            let received = result_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("large payload arrived");
            drop(out_tx);
            reader.join().unwrap();
            received
        });

        let mut peer = Peer::connect(address, &initiator_id, &responder_public).unwrap();
        let (out_tx, out_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let sender = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            out_tx.send(encoded).unwrap();
        });
        peer.run(out_rx, |_| {}).unwrap();
        sender.join().unwrap();

        assert_eq!(responder.join().unwrap(), expected);
    }
}
