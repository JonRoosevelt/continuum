use std::io::{Read, Write};

use snow::{HandshakeState, TransportState};

use crate::error::NetError;
use crate::identity::{Identity, NOISE_PARAMS};

const MAX_FRAME: u32 = 16 * 1024 * 1024;
const HANDSHAKE_BUF: usize = 2048;
const TAG_LEN: usize = 16;

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
        let mut buffer = vec![0u8; frame.len()];
        let read = self.state.read_message(&frame, &mut buffer)?;
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

fn write_frame<S: Write>(stream: &mut S, frame: &[u8]) -> Result<(), NetError> {
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
}
