use std::path::PathBuf;

use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, GetPropertyReply, ImageOrder,
    PropMode, SelectionNotifyEvent, SelectionRequestEvent, Time, Window, WindowClass,
    SELECTION_NOTIFY_EVENT,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use continuum_core::{
    content_hash, ClipItem, ContentHash, Representation, PLAIN_TEXT_MIME, PNG_MIME,
};

use crate::backend::{ClipboardBackend, ClipboardError};
use crate::files;

const PRIVATE_PROPERTY: &str = "CONTINUUM_SELECTION";

struct Atoms {
    clipboard: Atom,
    targets: Atom,
    utf8_string: Atom,
    text: Atom,
    string: Atom,
    plain: Atom,
    png: Atom,
    uri_list: Atom,
    gnome_copied_files: Atom,
    incr: Atom,
    timestamp: Atom,
    #[allow(dead_code)]
    multiple: Atom,
    property: Atom,
    sensitive: Atom,
}

pub struct X11Clipboard {
    conn: RustConnection,
    window: Window,
    atoms: Atoms,
    owned: Option<Vec<ClipItem>>,
    owned_files: Option<Vec<PathBuf>>,
    last_hash: Option<ContentHash>,
}

impl X11Clipboard {
    pub fn new() -> Result<Self, ClipboardError> {
        let (conn, screen_num) = x11rb::connect(None).map_err(|_| ClipboardError::Unavailable)?;

        let root = conn
            .setup()
            .roots
            .get(screen_num)
            .ok_or_else(|| ClipboardError::Read("X11 screen index out of range".into()))?
            .root;

        let window = conn
            .generate_id()
            .map_err(|err| ClipboardError::Read(format!("generate_id failed: {err}")))?;

        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(conn_err)?;

        let atoms = Atoms {
            clipboard: intern(&conn, "CLIPBOARD", false)?,
            targets: intern(&conn, "TARGETS", false)?,
            utf8_string: intern(&conn, "UTF8_STRING", false)?,
            text: intern(&conn, "TEXT", false)?,
            string: intern(&conn, "STRING", false)?,
            plain: intern(&conn, PLAIN_TEXT_MIME, false)?,
            png: intern(&conn, PNG_MIME, false)?,
            uri_list: intern(&conn, files::URI_LIST_MIME, false)?,
            gnome_copied_files: intern(&conn, files::GNOME_COPIED_FILES_MIME, false)?,
            incr: intern(&conn, "INCR", false)?,
            timestamp: intern(&conn, "TIMESTAMP", false)?,
            multiple: intern(&conn, "MULTIPLE", false)?,
            property: intern(&conn, PRIVATE_PROPERTY, false)?,
            sensitive: intern(&conn, "x-kde-passwordManagerHint", true)?,
        };

        conn.flush().map_err(conn_err)?;

        Ok(Self {
            conn,
            window,
            atoms,
            owned: None,
            owned_files: None,
            last_hash: None,
        })
    }

    fn handle_event(&mut self, event: Event) -> Result<(), ClipboardError> {
        match event {
            Event::SelectionRequest(request) => self.serve_request(request)?,
            Event::SelectionClear(clear) if clear.selection == self.atoms.clipboard => {
                self.owned = None;
                self.owned_files = None;
            }
            _ => {}
        }
        Ok(())
    }

    fn offered_atoms(&self) -> Vec<Atom> {
        let mut targets = vec![self.atoms.targets, self.atoms.timestamp];
        if self
            .owned_files
            .as_ref()
            .is_some_and(|files| !files.is_empty())
        {
            targets.push(self.atoms.uri_list);
            targets.push(self.atoms.gnome_copied_files);
            return targets;
        }
        let Some(owned) = self.owned.as_ref() else {
            return targets;
        };
        if owned
            .iter()
            .any(|item| item.representation(PLAIN_TEXT_MIME).is_some())
        {
            targets.extend([
                self.atoms.utf8_string,
                self.atoms.text,
                self.atoms.string,
                self.atoms.plain,
            ]);
        }
        if owned
            .iter()
            .any(|item| item.representation(PNG_MIME).is_some())
        {
            targets.push(self.atoms.png);
        }
        targets
    }

    fn served_bytes(&self, target: Atom) -> Option<Vec<u8>> {
        if let Some(files) = self.owned_files.as_ref() {
            if !files.is_empty() {
                if target == self.atoms.uri_list {
                    return Some(files::uri_list_bytes(files));
                }
                if target == self.atoms.gnome_copied_files {
                    return Some(files::gnome_copied_files_bytes(files));
                }
            }
        }
        let owned = self.owned.as_ref()?;
        for item in owned {
            for representation in &item.representations {
                match representation.mime.as_str() {
                    PLAIN_TEXT_MIME => {
                        if target == self.atoms.plain
                            || target == self.atoms.utf8_string
                            || target == self.atoms.text
                            || target == self.atoms.string
                        {
                            return Some(representation.bytes.clone());
                        }
                    }
                    PNG_MIME if target == self.atoms.png => {
                        return Some(representation.bytes.clone())
                    }
                    _ => {}
                }
            }
        }
        None
    }

    fn serve_request(&mut self, request: SelectionRequestEvent) -> Result<(), ClipboardError> {
        let property = if request.property == x11rb::NONE {
            request.target
        } else {
            request.property
        };

        let mut served = false;
        if request.target == self.atoms.targets {
            let targets = self.offered_atoms();
            self.conn
                .change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    AtomEnum::ATOM,
                    &targets,
                )
                .map_err(conn_err)?;
            served = true;
        } else if let Some(bytes) = self.served_bytes(request.target) {
            // X11 INCR writes are not implemented in v1; payloads are sent inline only.
            self.conn
                .change_property8(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    request.target,
                    &bytes,
                )
                .map_err(conn_err)?;
            served = true;
        }

        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: if served { property } else { x11rb::NONE },
        };
        self.conn
            .send_event(false, request.requestor, EventMask::NO_EVENT, notify)
            .map_err(conn_err)?;
        self.conn.flush().map_err(conn_err)?;
        Ok(())
    }

    fn exchange(
        &mut self,
        selection: Atom,
        target: Atom,
    ) -> Result<Option<GetPropertyReply>, ClipboardError> {
        let property = self.atoms.property;
        self.conn
            .convert_selection(self.window, selection, target, property, Time::CURRENT_TIME)
            .map_err(conn_err)?;
        self.conn.flush().map_err(conn_err)?;

        loop {
            let event = match self.conn.poll_for_event().map_err(conn_err)? {
                Some(event) => event,
                None => self.conn.wait_for_event().map_err(conn_err)?,
            };

            if let Event::SelectionNotify(notify) = &event {
                if notify.requestor == self.window
                    && notify.selection == selection
                    && notify.target == target
                {
                    let reply_property = notify.property;
                    if reply_property == x11rb::NONE {
                        return Ok(None);
                    }
                    let reply = self
                        .conn
                        .get_property(
                            true,
                            self.window,
                            reply_property,
                            AtomEnum::ANY,
                            0,
                            u32::MAX / 4,
                        )
                        .map_err(conn_err)?
                        .reply()
                        .map_err(reply_err)?;
                    return Ok(Some(reply));
                }
            }

            self.handle_event(event)?;
        }
    }

    fn fetch_bytes(
        &mut self,
        selection: Atom,
        target: Atom,
    ) -> Result<Option<Vec<u8>>, ClipboardError> {
        let Some(reply) = self.exchange(selection, target)? else {
            return Ok(None);
        };
        if reply.type_ == self.atoms.incr {
            // X11 INCR reads are not implemented in v1; large payloads are skipped.
            return Ok(None);
        }
        Ok((!reply.value.is_empty()).then_some(reply.value))
    }

    fn owner_targets(&mut self) -> Result<Vec<Atom>, ClipboardError> {
        let selection = self.atoms.clipboard;
        let Some(reply) = self.exchange(selection, self.atoms.targets)? else {
            return Ok(Vec::new());
        };
        if reply.type_ != u32::from(AtomEnum::ATOM) {
            return Ok(Vec::new());
        }
        Ok(self.atoms_from_property(&reply.value))
    }

    fn fetch_file_items(
        &mut self,
        target: Atom,
        parse: fn(&[u8]) -> Vec<PathBuf>,
    ) -> Result<Option<Vec<ClipItem>>, ClipboardError> {
        let Some(bytes) = self.fetch_bytes(self.atoms.clipboard, target)? else {
            return Ok(None);
        };
        let mut items = Vec::new();
        for path in parse(&bytes) {
            if let Some((name, content)) = files::read_file(&path) {
                items.push(ClipItem::new(vec![files::encode(&name, &content)])?);
            }
        }
        Ok((!items.is_empty()).then_some(items))
    }

    fn write_files(&mut self, items: &[ClipItem]) -> Result<(), ClipboardError> {
        let mut paths = Vec::new();
        let mut saved = Vec::new();
        for item in items {
            let Some((name, content)) = files::decode(&item.representations[0]) else {
                continue;
            };
            let path = files::save(&name, &content)
                .map_err(|err| ClipboardError::Write(err.to_string()))?;
            let saved_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("file")
                .to_string();
            saved.push(ClipItem::new(vec![files::encode(&saved_name, &content)])?);
            paths.push(path);
        }
        if paths.is_empty() {
            return Err(ClipboardError::Write("no files could be saved".into()));
        }

        self.owned = None;
        self.owned_files = Some(paths);
        self.conn
            .set_selection_owner(self.window, self.atoms.clipboard, Time::CURRENT_TIME)
            .map_err(conn_err)?;
        self.conn.flush().map_err(conn_err)?;

        // Read-back yields each saved file's basename and bytes, so seeding this exact shape
        // stops a clipboard manager re-offering our own file list from echoing back.
        self.last_hash = Some(content_hash(&saved));
        Ok(())
    }

    fn atoms_from_property(&self, bytes: &[u8]) -> Vec<Atom> {
        let little_endian = self.conn.setup().image_byte_order == ImageOrder::LSB_FIRST;
        bytes
            .chunks_exact(4)
            .map(|chunk| {
                let raw = [chunk[0], chunk[1], chunk[2], chunk[3]];
                if little_endian {
                    u32::from_le_bytes(raw)
                } else {
                    u32::from_be_bytes(raw)
                }
            })
            .collect()
    }
}

impl ClipboardBackend for X11Clipboard {
    fn read(&mut self) -> Result<Option<Vec<ClipItem>>, ClipboardError> {
        let Some(items) = self.read_current()? else {
            return Ok(None);
        };
        // X11 (without XFixes) exposes no change counter; content hashing is the shared
        // change-detection mechanism, and write() seeds the same hash to suppress echoes.
        let hash = content_hash(&items);
        if self.last_hash == Some(hash) {
            return Ok(None);
        }
        self.last_hash = Some(hash);
        Ok(Some(items))
    }

    fn read_current(&mut self) -> Result<Option<Vec<ClipItem>>, ClipboardError> {
        while let Some(event) = self.conn.poll_for_event().map_err(conn_err)? {
            self.handle_event(event)?;
        }

        let owner = self
            .conn
            .get_selection_owner(self.atoms.clipboard)
            .map_err(conn_err)?
            .reply()
            .map_err(reply_err)?
            .owner;
        if owner == x11rb::NONE || owner == self.window {
            return Ok(None);
        }

        let targets = self.owner_targets()?;
        if self.atoms.sensitive != x11rb::NONE && targets.contains(&self.atoms.sensitive) {
            return Ok(None);
        }

        if targets.contains(&self.atoms.uri_list) {
            if let Some(items) =
                self.fetch_file_items(self.atoms.uri_list, files::parse_uri_list)?
            {
                return Ok(Some(items));
            }
        } else if targets.contains(&self.atoms.gnome_copied_files) {
            if let Some(items) = self.fetch_file_items(
                self.atoms.gnome_copied_files,
                files::parse_gnome_copied_files,
            )? {
                return Ok(Some(items));
            }
        }

        let mut representations = Vec::new();
        for target in [self.atoms.utf8_string, self.atoms.text, self.atoms.string] {
            if let Some(bytes) = self.fetch_bytes(self.atoms.clipboard, target)? {
                if !bytes.is_empty() {
                    representations.push(Representation::new(PLAIN_TEXT_MIME, bytes));
                    break;
                }
            }
        }
        if let Some(bytes) = self.fetch_bytes(self.atoms.clipboard, self.atoms.png)? {
            if !bytes.is_empty() {
                representations.push(Representation::new(PNG_MIME, bytes));
            }
        }

        if representations.is_empty() {
            return Ok(None);
        }
        Ok(Some(vec![ClipItem::new(representations)?]))
    }

    fn write(&mut self, items: &[ClipItem]) -> Result<(), ClipboardError> {
        if items.is_empty() {
            return Err(ClipboardError::Write(
                "clipboard payload has no items".into(),
            ));
        }

        let files_only = items.iter().all(|item| {
            item.representations.len() == 1 && files::is_file(&item.representations[0])
        });
        if files_only {
            return self.write_files(items);
        }

        self.owned_files = None;
        self.owned = Some(items.to_vec());
        self.conn
            .set_selection_owner(self.window, self.atoms.clipboard, Time::CURRENT_TIME)
            .map_err(conn_err)?;
        self.conn.flush().map_err(conn_err)?;

        self.last_hash = Some(content_hash(items));
        Ok(())
    }
}

fn intern(conn: &RustConnection, name: &str, only_if_exists: bool) -> Result<Atom, ClipboardError> {
    conn.intern_atom(only_if_exists, name.as_bytes())
        .map_err(conn_err)?
        .reply()
        .map_err(reply_err)
        .map(|reply| reply.atom)
}

fn conn_err(err: x11rb::errors::ConnectionError) -> ClipboardError {
    ClipboardError::Read(err.to_string())
}

fn reply_err(err: x11rb::errors::ReplyError) -> ClipboardError {
    ClipboardError::Read(err.to_string())
}
