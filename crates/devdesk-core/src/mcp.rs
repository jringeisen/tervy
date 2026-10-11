//! Local MCP stdio endpoint and bounded request/reply bridge to the supervisor.
//! The bridge never opens state.db; the supervisor checks every operation.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

const MAX_MESSAGE: u64 = 524_288;
pub const MAX_MCP_NOTES: usize = 128 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeetingEdit {
    pub id: String,
    pub expected_revision: u64,
    pub title: Option<String>,
    pub notes: Option<String>,
}

impl MeetingEdit {
    pub fn validate(&self) -> Result<(), String> {
        if uuid::Uuid::parse_str(&self.id).is_err() || self.expected_revision == 0 {
            return Err("Edit requires a meeting ID and current revision".into());
        }
        if self.title.is_none() && self.notes.is_none() {
            return Err("Provide a title or notes to edit".into());
        }
        if self.title.as_ref().is_some_and(|title| {
            let title = title.trim();
            title.is_empty() || title.len() > 160 || title.contains(['\n', '\r', '\0'])
        }) {
            return Err("Meeting title must be one line, 1–160 bytes".into());
        }
        if self
            .notes
            .as_ref()
            .is_some_and(|notes| notes.len() > MAX_MCP_NOTES || notes.contains('\0'))
        {
            return Err("MCP meeting notes must be at most 128 KiB and contain no NUL".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    List {
        query: String,
        limit: usize,
        offset: usize,
    },
    Read {
        id: String,
    },
    Write {
        write: crate::prompts::Write,
    },
    Delete {
        id: String,
        revision: i64,
    },
    ListMeetings {
        query: String,
        limit: usize,
        offset: usize,
    },
    ReadMeeting {
        id: String,
    },
    EditMeeting {
        edit: MeetingEdit,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub agent_id: String,
    pub launch: u64,
    pub token: String,
    pub operation: Operation,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Reply {
    pub result: Option<Value>,
    pub error: Option<String>,
}

pub struct Request {
    pub envelope: Envelope,
    pub reply: mpsc::SyncSender<Reply>,
}

pub struct Server {
    pub path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    pub fn bind(path: PathBuf, send: impl Fn(Request) + Send + Sync + 'static) -> io::Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("Missing MCP socket directory"))?;
        let meta = std::fs::symlink_metadata(parent)?;
        if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::other(
                "MCP socket directory is not owned by this user",
            ));
        }
        if let Ok(meta) = std::fs::symlink_metadata(&path) {
            if !meta.file_type().is_socket() || meta.uid() != unsafe { libc::geteuid() } {
                return Err(io::Error::other(
                    "Refusing to replace a non-owned MCP socket",
                ));
            }
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let send = Arc::new(send);
        let active = Arc::new(AtomicUsize::new(0));
        let thread = std::thread::Builder::new()
            .name("tervy-mcp-ipc".into())
            .spawn(move || {
                for incoming in listener.incoming() {
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(mut stream) = incoming else { continue };
                    if active.load(Ordering::Acquire) >= 16 {
                        continue;
                    }
                    active.fetch_add(1, Ordering::AcqRel);
                    let active_worker = active.clone();
                    let send = send.clone();
                    if std::thread::Builder::new()
                        .name("tervy-mcp-call".into())
                        .spawn(move || {
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                            let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                            let response = (|| -> Result<Reply, String> {
                                let mut bytes = Vec::new();
                                Read::by_ref(&mut stream)
                                    .take(MAX_MESSAGE + 1)
                                    .read_to_end(&mut bytes)
                                    .map_err(|e| e.to_string())?;
                                if bytes.len() as u64 > MAX_MESSAGE {
                                    return Err("MCP request too large".into());
                                }
                                let envelope: Envelope =
                                    serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                                let (reply, receive) = mpsc::sync_channel(1);
                                send(Request { envelope, reply });
                                receive
                                    .recv_timeout(Duration::from_secs(95))
                                    .map_err(|_| "Tervy did not respond to MCP request".to_string())
                            })()
                            .unwrap_or_else(|error| Reply {
                                result: None,
                                error: Some(error),
                            });
                            if let Ok(bytes) = serde_json::to_vec(&response) {
                                let _ = stream.write_all(&bytes);
                            }
                            active_worker.fetch_sub(1, Ordering::AcqRel);
                        })
                        .is_err()
                    {
                        active.fetch_sub(1, Ordering::AcqRel);
                    }
                }
            })?;
        Ok(Self {
            path,
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = UnixStream::connect(&self.path);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
}

pub fn meeting_summary(meeting: &crate::meetings::Meeting) -> Value {
    json!({
        "id": meeting.id,
        "title": meeting.title,
        "started_at": meeting.started_at,
        "duration_secs": meeting.duration_secs,
        "status": meeting.status,
        "revision": meeting.revision,
        "has_notes": !meeting.notes.is_empty(),
        "has_transcript": !meeting.transcript.is_empty(),
    })
}

pub fn meeting_detail(meeting: &crate::meetings::Meeting) -> Value {
    json!({
        "id": meeting.id,
        "title": meeting.title,
        "started_at": meeting.started_at,
        "duration_secs": meeting.duration_secs,
        "status": meeting.status,
        "revision": meeting.revision,
        "notes": meeting.notes,
        "has_transcript": !meeting.transcript.is_empty(),
    })
}

fn tools(read_only: bool) -> Vec<Value> {
    let mut result = vec![
        tool(
            "list_prompts",
            "Search the shared Tervy prompt library. Returns titles, IDs and revisions, not full bodies.",
            json!({"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100},"offset":{"type":"integer","minimum":0}}),
            &[],
        ),
        tool(
            "read_prompt",
            "Read a saved Tervy prompt by ID.",
            json!({"id":{"type":"string"}}),
            &["id"],
        ),
        tool(
            "list_meetings",
            "Search meeting titles and notes. Returns metadata and revisions, not full notes or audio paths.",
            json!({"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100},"offset":{"type":"integer","minimum":0}}),
            &[],
        ),
        tool(
            "read_meeting",
            "Read meeting notes and metadata by ID. Does not return the transcript or audio.",
            json!({"id":{"type":"string"}}),
            &["id"],
        ),
    ];
    if !read_only {
        result.push(tool("write_prompt", "Create or update a shared Tervy prompt. For an update, supply id and expected_revision from read_prompt. Return the new revision.", json!({"id":{"type":"string"},"title":{"type":"string"},"body":{"type":"string"},"expected_revision":{"type":"integer","minimum":1}}), &["title","body"]));
        result.push(tool(
            "delete_prompt",
            "Request deletion of a saved Tervy prompt. Tervy asks the user to confirm it.",
            json!({"id":{"type":"string"},"revision":{"type":"integer","minimum":1}}),
            &["id", "revision"],
        ));
        result.push(tool(
            "edit_meeting",
            "Update a meeting title and/or Markdown notes. Supply expected_revision from read_meeting; the update fails if the meeting changed.",
            json!({"id":{"type":"string"},"expected_revision":{"type":"integer","minimum":1},"title":{"type":"string"},"notes":{"type":"string"}}),
            &["id", "expected_revision"],
        ));
    }
    result
}

fn call(operation: Operation) -> Reply {
    let result = (|| -> Result<Reply, String> {
        let envelope = Envelope {
            agent_id: std::env::var("DEVDESK_AGENT_ID").map_err(|_| "Missing agent identity")?,
            launch: std::env::var("DEVDESK_LAUNCH_ID")
                .map_err(|_| "Missing launch identity")?
                .parse()
                .map_err(|_| "Invalid launch identity")?,
            token: std::env::var("DEVDESK_HOOK_TOKEN").map_err(|_| "Missing agent token")?,
            operation,
        };
        let path = std::env::var_os("DEVDESK_MCP_SOCKET").ok_or("Missing Tervy MCP socket")?;
        let mut stream = UnixStream::connect(Path::new(&path)).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(100)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        stream
            .write_all(&serde_json::to_vec(&envelope).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        stream
            .shutdown(std::net::Shutdown::Write)
            .map_err(|e| e.to_string())?;
        let mut response = Vec::new();
        stream
            .take(MAX_MESSAGE + 1)
            .read_to_end(&mut response)
            .map_err(|e| e.to_string())?;
        if response.len() as u64 > MAX_MESSAGE {
            return Err("Tervy MCP reply too large".into());
        }
        serde_json::from_slice(&response).map_err(|e| e.to_string())
    })();
    result.unwrap_or_else(|error| Reply {
        result: None,
        error: Some(error),
    })
}

/// Invoked before GUI startup as `tervy mcp`. Stdout is reserved for JSON-RPC.
pub fn run_stdio() {
    let read_only = std::env::args().any(|argument| argument == "--read-only")
        || std::env::var("DEVDESK_MCP_ROLE").as_deref() == Ok("read");
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let mut stdout = io::stdout().lock();
    while let Ok(Some(line)) = read_bounded_line(&mut stdin) {
        let Ok(message) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        let Some(id) = message.get("id") else {
            continue;
        };
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let response = match method {
            "initialize" => {
                let requested = message
                    .pointer("/params/protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let version = if ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"]
                    .contains(&requested)
                {
                    requested
                } else {
                    "2025-11-25"
                };
                json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":version,"capabilities":{"tools":{}},"serverInfo":{"name":"tervy","version":env!("CARGO_PKG_VERSION")}}})
            }
            "ping" => json!({"jsonrpc":"2.0","id":id,"result":{}}),
            "tools/list" => json!({"jsonrpc":"2.0","id":id,"result":{"tools":tools(read_only)}}),
            "tools/call" => {
                let params = message.get("params").cloned().unwrap_or(Value::Null);
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let operation = match name {
                    "list_prompts" => Ok(Operation::List {
                        query: args
                            .get("query")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        limit: args
                            .get("limit")
                            .and_then(Value::as_u64)
                            .unwrap_or(50)
                            .min(100) as usize,
                        offset: args
                            .get("offset")
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                            .min(100_000) as usize,
                    }),
                    "read_prompt" => args
                        .get("id")
                        .and_then(Value::as_str)
                        .map(|id| Operation::Read { id: id.to_string() })
                        .ok_or("Missing prompt ID"),
                    "list_meetings" => Ok(Operation::ListMeetings {
                        query: args
                            .get("query")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        limit: args
                            .get("limit")
                            .and_then(Value::as_u64)
                            .unwrap_or(50)
                            .clamp(1, 100) as usize,
                        offset: args
                            .get("offset")
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                            .min(100_000) as usize,
                    }),
                    "read_meeting" => args
                        .get("id")
                        .and_then(Value::as_str)
                        .map(|id| Operation::ReadMeeting { id: id.to_string() })
                        .ok_or("Missing meeting ID"),
                    "write_prompt" if !read_only => {
                        serde_json::from_value::<crate::prompts::Write>(args)
                            .map(|write| Operation::Write { write })
                            .map_err(|_| "Invalid prompt input")
                    }
                    "delete_prompt" if !read_only => serde_json::from_value::<DeleteArgs>(args)
                        .map(|args| Operation::Delete {
                            id: args.id,
                            revision: args.revision,
                        })
                        .map_err(|_| "Invalid delete input"),
                    "edit_meeting" if !read_only => serde_json::from_value::<MeetingEdit>(args)
                        .map_err(|_| "Invalid meeting edit input")
                        .and_then(|edit| {
                            edit.validate().map_err(|_| "Invalid meeting edit input")?;
                            Ok(Operation::EditMeeting { edit })
                        }),
                    _ => Err("Unknown or unavailable tool"),
                };
                let reply = operation.map(call).unwrap_or_else(|error| Reply {
                    result: None,
                    error: Some(error.into()),
                });
                if let Some(error) = reply.error {
                    json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":error}],"isError":true}})
                } else {
                    let content = reply.result.unwrap_or(Value::Null);
                    json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":content.to_string()}],"structuredContent":{"data":content}}})
                }
            }
            _ => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}})
            }
        };
        if serde_json::to_writer(&mut stdout, &response).is_err()
            || stdout.write_all(b"\n").is_err()
            || stdout.flush().is_err()
        {
            break;
        }
    }
}

fn read_bounded_line<R: BufRead>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }
        let length = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |at| at + 1);
        if line.len() as u64 + length as u64 > MAX_MESSAGE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MCP message too large",
            ));
        }
        line.extend_from_slice(&available[..length]);
        reader.consume(length);
        if line.last() == Some(&b'\n') {
            return Ok(Some(line));
        }
    }
}

#[derive(Deserialize)]
struct DeleteArgs {
    id: String,
    revision: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_list_exposes_only_reads_for_side_sessions() {
        let read: Vec<_> = tools(true)
            .into_iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect();
        let write: Vec<_> = tools(false)
            .into_iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            read,
            [
                "list_prompts",
                "read_prompt",
                "list_meetings",
                "read_meeting"
            ]
        );
        assert_eq!(
            write,
            [
                "list_prompts",
                "read_prompt",
                "list_meetings",
                "read_meeting",
                "write_prompt",
                "delete_prompt",
                "edit_meeting"
            ]
        );
    }

    #[test]
    fn meeting_edit_requires_a_current_revision_and_valid_content() {
        let id = uuid::Uuid::new_v4().to_string();
        let edit = MeetingEdit {
            id,
            expected_revision: 2,
            title: Some("Notes review".into()),
            notes: Some("## Decisions\n\nNone yet.".into()),
        };
        assert!(edit.validate().is_ok());
        assert!(
            MeetingEdit {
                expected_revision: 0,
                ..edit.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            MeetingEdit {
                title: None,
                notes: None,
                ..edit.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            MeetingEdit {
                title: Some("Bad\nTitle".into()),
                ..edit.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            MeetingEdit {
                notes: Some("x".repeat(MAX_MCP_NOTES + 1)),
                ..edit
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn bounded_stdio_lines_reject_large_inputs() {
        let data = b"{\"id\":1}\n{\"id\":2}\n";
        let mut input = io::BufReader::new(&data[..]);
        assert_eq!(
            read_bounded_line(&mut input).unwrap().unwrap(),
            b"{\"id\":1}\n"
        );
        assert_eq!(
            read_bounded_line(&mut input).unwrap().unwrap(),
            b"{\"id\":2}\n"
        );
        assert!(read_bounded_line(&mut input).unwrap().is_none());
        let large = vec![b'a'; MAX_MESSAGE as usize + 1];
        assert!(read_bounded_line(&mut io::BufReader::new(&large[..])).is_err());
    }

    #[test]
    fn local_socket_round_trips_one_request_and_reply() {
        // Unix socket paths are short on macOS; the process temp directory can
        // already consume the entire limit before the socket name is appended.
        let directory =
            PathBuf::from("/tmp").join(format!("tm-{}", &uuid::Uuid::new_v4().to_string()[..8]));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("mcp.sock");
        let server = Server::bind(path.clone(), |request| {
            assert!(matches!(request.envelope.operation, Operation::List { .. }));
            let _ = request.reply.send(Reply {
                result: Some(json!({"items":[]})),
                error: None,
            });
        })
        .unwrap();
        let mut stream = UnixStream::connect(&server.path).unwrap();
        stream
            .write_all(
                &serde_json::to_vec(&Envelope {
                    agent_id: uuid::Uuid::new_v4().to_string(),
                    launch: 1,
                    token: "token".into(),
                    operation: Operation::List {
                        query: "".into(),
                        limit: 10,
                        offset: 0,
                    },
                })
                .unwrap(),
            )
            .unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        let reply: Reply = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(reply.result.unwrap()["items"], json!([]));
        drop(server);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
