use std::io::{Read, Write};

use snow::{HandshakeState, TransportState};

use crate::error::NetError;
use crate::identity::{Identity, NOISE_PARAMS};

const MAX_FRAME: u32 = 16 * 1024 * 1024;
const HANDSHAKE_BUF: usize = 2048;
const TAG_LEN: usize = 16;
const XX_PARAMS: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

pub struct Session {
    state: TransportState,
}

impl Session {
    pub fn send<S: Write>(&mut self, stream: &mut S, payload: &[u8]) -> Result<(), NetError> {
        let mut buffer = vec![0u8; payload.len() + TAG_LEN];
        let written = self.state.write_message(payload, &mut buffer)?;
        write_frame(stream, &buffer[..written])
    }

    pub fn recv<S: Read>(&mut self, stream: &mut S) -> Result<Vec<u8>, NetError> {
        let frame = read_frame(stream)?;
        self.open(&frame)
    }

    /// Encrypts `payload` into a single transport message (no length prefix).
    pub fn seal(&mut self, payload: &[u8]) -> Result<Vec<u8>, NetError> {
        let mut buffer = vec![0u8; payload.len() + TAG_LEN];
        let written = self.state.write_message(payload, &mut buffer)?;
        buffer.truncate(written);
        Ok(buffer)
    }

    /// Decrypts a single transport message previously produced by [`Session::seal`].
    pub fn open(&mut self, frame: &[u8]) -> Result<Vec<u8>, NetError> {
        let mut buffer = vec![0u8; frame.len()];
        let read = self.state.read_message(frame, &mut buffer)?;
        buffer.truncate(read);
        Ok(buffer)
    }

    /// Static public key the peer proved ownership of during the handshake.
    #[must_use]
    pub fn remote_static(&self) -> Option<&[u8]> {
        self.state.get_remote_static()
    }
}

pub fn initiate<S: Read + Write>(
    stream: &mut S,
    identity: &Identity,
    remote_public: &[u8],
) -> Result<Session, NetError> {
    let mut handshake = handshake_state(true, identity, Some(remote_public))?;

    let mut buffer = vec![0u8; HANDSHAKE_BUF];
    let written = handshake.write_message(&[], &mut buffer)?;
    write_frame(stream, &buffer[..written])?;

    let reply = read_frame(stream)?;
    handshake.read_message(&reply, &mut buffer)?;

    Ok(Session {
        state: handshake.into_transport_mode()?,
    })
}

pub fn respond<S: Read + Write>(stream: &mut S, identity: &Identity) -> Result<Session, NetError> {
    let mut handshake = handshake_state(false, identity, None)?;

    let request = read_frame(stream)?;
    let mut buffer = vec![0u8; HANDSHAKE_BUF];
    handshake.read_message(&request, &mut buffer)?;

    let written = handshake.write_message(&[], &mut buffer)?;
    write_frame(stream, &buffer[..written])?;

    Ok(Session {
        state: handshake.into_transport_mode()?,
    })
}

/// XX handshake for pairing: authenticates both static keys without prior knowledge.
pub fn xx_initiate<S: Read + Write>(
    stream: &mut S,
    identity: &Identity,
) -> Result<Session, NetError> {
    let params = XX_PARAMS.parse()?;
    let mut handshake = snow::Builder::new(params)
        .local_private_key(identity.private_key())
        .build_initiator()?;

    let mut buffer = vec![0u8; HANDSHAKE_BUF];
    let written = handshake.write_message(&[], &mut buffer)?;
    write_frame(stream, &buffer[..written])?;

    let reply = read_frame(stream)?;
    handshake.read_message(&reply, &mut buffer)?;

    let written = handshake.write_message(&[], &mut buffer)?;
    write_frame(stream, &buffer[..written])?;

    Ok(Session {
        state: handshake.into_transport_mode()?,
    })
}

pub fn xx_respond<S: Read + Write>(
    stream: &mut S,
    identity: &Identity,
) -> Result<Session, NetError> {
    let params = XX_PARAMS.parse()?;
    let mut handshake = snow::Builder::new(params)
        .local_private_key(identity.private_key())
        .build_responder()?;

    let mut buffer = vec![0u8; HANDSHAKE_BUF];
    let request = read_frame(stream)?;
    handshake.read_message(&request, &mut buffer)?;

    let written = handshake.write_message(&[], &mut buffer)?;
    write_frame(stream, &buffer[..written])?;

    let finalize = read_frame(stream)?;
    handshake.read_message(&finalize, &mut buffer)?;

    Ok(Session {
        state: handshake.into_transport_mode()?,
    })
}

/// Six-digit code derived from both static keys; equal on both ends, detects a MITM.
#[must_use]
pub fn short_authentication_string(a: &[u8], b: &[u8]) -> String {
    let (first, second) = if a <= b { (a, b) } else { (b, a) };
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"continuum-pairing-sas");
    hasher.update(first);
    hasher.update(second);
    let digest = hasher.finalize();
    let bytes = digest.as_bytes();
    let value = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % 1_000_000;
    format!("{value:06}")
}

fn handshake_state(
    initiator: bool,
    identity: &Identity,
    remote_public: Option<&[u8]>,
) -> Result<HandshakeState, NetError> {
    let params = NOISE_PARAMS.parse()?;
    let mut builder = snow::Builder::new(params).local_private_key(identity.private_key());
    if let Some(remote) = remote_public {
        builder = builder.remote_public_key(remote);
    }
    if initiator {
        Ok(builder.build_initiator()?)
    } else {
        Ok(builder.build_responder()?)
    }
}

pub(crate) fn write_frame<S: Write>(stream: &mut S, frame: &[u8]) -> Result<(), NetError> {
    let len = u32::try_from(frame.len()).map_err(|_| NetError::FrameTooLarge(u32::MAX))?;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(frame)?;
    stream.flush()?;
    Ok(())
}

fn read_frame<S: Read>(stream: &mut S) -> Result<Vec<u8>, NetError> {
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes)?;
    let len = u32::from_be_bytes(len_bytes);
    if len > MAX_FRAME {
        return Err(NetError::FrameTooLarge(len));
    }
    let mut frame = vec![0u8; len as usize];
    stream.read_exact(&mut frame)?;
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use std::net::{TcpListener, TcpStream};

    use super::*;

    #[test]
    fn handshake_authenticates_and_roundtrips() {
        let responder_id = Identity::generate().unwrap();
        let initiator_id = Identity::generate().unwrap();
        let responder_public = responder_id.public_key().to_vec();
        let expected_initiator = initiator_id.public_key().to_vec();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let responder = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut session = respond(&mut stream, &responder_id).unwrap();
            assert_eq!(session.remote_static().unwrap(), expected_initiator);
            let message = session.recv(&mut stream).unwrap();
            session.send(&mut stream, &message).unwrap();
        });

        let mut stream = TcpStream::connect(address).unwrap();
        let mut session = initiate(&mut stream, &initiator_id, &responder_public).unwrap();
        assert_eq!(session.remote_static().unwrap(), responder_public);
        session.send(&mut stream, b"hello world").unwrap();
        assert_eq!(session.recv(&mut stream).unwrap(), b"hello world");

        responder.join().unwrap();
    }

    #[test]
    fn xx_pairing_agrees_on_authentication_string() {
        let alice = Identity::generate().unwrap();
        let bob = Identity::generate().unwrap();
        let alice_public = alice.public_key().to_vec();
        let bob_public = bob.public_key().to_vec();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let bob_key = bob_public.clone();

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut session = xx_respond(&mut stream, &bob).unwrap();
            let remote = session.remote_static().unwrap().to_vec();
            let code = short_authentication_string(&remote, &bob_key);
            let message = session.recv(&mut stream).unwrap();
            session.send(&mut stream, &message).unwrap();
            code
        });

        let mut stream = TcpStream::connect(address).unwrap();
        let mut session = xx_initiate(&mut stream, &alice).unwrap();
        let remote = session.remote_static().unwrap().to_vec();
        assert_eq!(remote, bob_public);
        let code = short_authentication_string(&alice_public, &remote);
        session.send(&mut stream, b"hi").unwrap();
        assert_eq!(session.recv(&mut stream).unwrap(), b"hi");

        assert_eq!(code, handle.join().unwrap());
    }
}
