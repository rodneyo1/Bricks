//! Current-user/session-restricted local named pipes. Token checks are performed
//! on both sides. An obscure pipe name is never treated as authentication.
use crate::{
    Result,
    protocol::{self, AgentState, Command, Failure, Frame, Outcome, Request},
    service::Service,
};
use sha2::{Digest, Sha256};
use std::{
    ffi::c_void,
    os::windows::io::AsRawHandle,
    path::Path,
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncWrite, WriteHalf},
    net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions},
    sync::{Semaphore, mpsc},
    time::timeout,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LocalFree},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        },
        GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    System::{
        Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId},
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn identity(pid: u32) -> Result<(String, u32)> {
    // SAFETY: Handles are checked, buffers are aligned and length-checked, and all
    // token/SID pointers are consumed while their backing allocation is alive.
    unsafe {
        let process = if pid == std::process::id() {
            None
        } else {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            Some(Handle(h))
        };
        let raw = process.as_ref().map_or(GetCurrentProcess(), |p| p.0);
        let mut token = ptr::null_mut();
        if OpenProcessToken(raw, TOKEN_QUERY, &mut token) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let token = Handle(token);
        let mut length = 0;
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut length);
        if length == 0 || length > 65536 {
            return Err("invalid Windows token size".into());
        }
        let mut buffer = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
        if GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            length,
            &mut length,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut sid = ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut end = 0;
        while *sid.add(end) != 0 {
            end += 1;
        }
        let name = String::from_utf16(std::slice::from_raw_parts(sid, end));
        LocalFree(sid.cast());
        let name = name?;
        let mut session = 0;
        if ProcessIdToSessionId(pid, &mut session) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok((name, session))
    }
}
pub fn pipe_name(root: &Path) -> Result<String> {
    let (sid, session) = identity(std::process::id())?;
    let root = root.canonicalize()?;
    let digest = format!(
        "{:x}",
        Sha256::digest(root.to_string_lossy().to_lowercase().as_bytes())
    );
    Ok(format!(
        r"\\.\pipe\Bricks-{sid}-{session}-{}",
        &digest[..24]
    ))
}
fn verify_peer(handle: *mut c_void, server_side: bool) -> Result<()> {
    let mut pid = 0;
    // SAFETY: The caller holds the connected pipe throughout this query.
    let success = unsafe {
        if server_side {
            GetNamedPipeClientProcessId(handle, &mut pid)
        } else {
            GetNamedPipeServerProcessId(handle, &mut pid)
        }
    };
    if success == 0 || pid == 0 || identity(pid)? != identity(std::process::id())? {
        return Err(Box::new(Failure::new(
            "PermissionDenied",
            "Local peer must be the same Windows user and interactive session.",
        )));
    }
    Ok(())
}
fn listener(name: &str, first: bool) -> Result<NamedPipeServer> {
    let (sid, _) = identity(std::process::id())?;
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = ptr::null_mut();
    // SAFETY: A valid SDDL buffer is converted by Windows. The resulting security
    // descriptor lives through CreateNamedPipe and is then released with LocalFree.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let result = ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .max_instances(5)
            .in_buffer_size(65536)
            .out_buffer_size(65536)
            .create_with_security_attributes_raw(
                name,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            );
        LocalFree(descriptor);
        Ok(result?)
    }
}
async fn write<W: AsyncWrite + Unpin, T: serde::Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<()> {
    timeout(Duration::from_secs(3), protocol::write_frame(writer, value)).await?
}

pub async fn serve(
    root: &Path,
    service: Service,
    stop: Arc<AtomicBool>,
    idle_exit: Option<Duration>,
) -> Result<()> {
    struct ConnectionOwner {
        service: Service,
        id: u64,
    }
    impl Drop for ConnectionOwner {
        fn drop(&mut self) {
            self.service.cancel_owner(self.id);
        }
    }
    let name = pipe_name(root)?;
    let mut pending = listener(&name, true)?;
    let slots = Arc::new(Semaphore::new(4));
    let ids = AtomicU64::new(1);
    let mut tasks = tokio::task::JoinSet::new();
    let mut idle_since = Instant::now();
    loop {
        tokio::select! {
            result=pending.connect(), if slots.available_permits()>0=>{
                result?;
                if verify_peer(pending.as_raw_handle(),true).is_err() {pending.disconnect()?;continue;}
                let permit=slots.clone().acquire_owned().await?; let pipe=pending;
                // Keep a listening instance available while existing peers run.
                // Four admitted peers plus one bounded pending listener.
                let owner=ids.fetch_add(1,Ordering::Relaxed);let controller=service.clone();
                tasks.spawn(async move {let _permit=permit;let _owner=ConnectionOwner{service:controller.clone(),id:owner};connection(pipe,controller,owner).await});
                pending=listener(&name,false)?; idle_since=Instant::now();
            }
            _=tokio::time::sleep(Duration::from_millis(100))=>{
                while tasks.try_join_next().is_some() {}
                if stop.load(Ordering::Acquire) {break;}
                if slots.available_permits()==4 && service.snapshot().active_operations.is_empty() {
                    if idle_exit.is_some_and(|limit|idle_since.elapsed()>=limit) {break;}
                } else {idle_since=Instant::now();}
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}
async fn connection(pipe: NamedPipeServer, service: Service, owner: u64) -> Result<()> {
    let (mut reader, mut writer) = tokio::io::split(pipe);
    let hello: Request =
        timeout(Duration::from_secs(3), protocol::read_frame(&mut reader)).await??;
    if !matches!(
        hello,
        Request::Hello {
            version: protocol::VERSION
        }
    ) {
        write(
            &mut writer,
            &Frame::Rejected {
                id: 0,
                error: Failure::new(
                    "IncompatibleVersion",
                    "This agent requires control API version 1.",
                ),
            },
        )
        .await?;
        return Ok(());
    }
    let (state, mut events) = service.attach();
    write(
        &mut writer,
        &Frame::Welcome {
            version: protocol::VERSION,
            state: Box::new(state),
        },
    )
    .await?;
    let (requests, mut incoming) = mpsc::channel(16);
    let reader_task = tokio::spawn(async move {
        loop {
            let result = protocol::read_frame::<Request>(&mut reader).await;
            let failed = result.is_err();
            if requests.send(result).await.is_err() || failed {
                break;
            }
        }
    });
    struct Abort(tokio::task::JoinHandle<()>);
    impl Drop for Abort {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _reader = Abort(reader_task);
    let mut last_id = 0;
    type Completion = tokio::sync::oneshot::Receiver<std::result::Result<Outcome, Failure>>;
    let mut active: Option<(u64, u64, Completion)> = None;
    loop {
        tokio::select! {
            event=events.recv()=>{write(&mut writer,&event?).await?;}
            request=incoming.recv()=>{
                let request=request.ok_or("local connection closed")??;
                match request {
                    Request::Execute{id,command}=>{
                        if id==0 || id<=last_id || active.is_some() {write(&mut writer,&Frame::Rejected{id,error:Failure::new("Busy","Request IDs must increase; one operation per connection is allowed.")}).await?;continue;}
                        last_id=id;
                        match service.execute(owner,command) {
                            Ok((operation,reply))=>{write(&mut writer,&Frame::Accepted{id,operation}).await?;active=Some((id,operation,reply));}
                            Err(error)=>write(&mut writer,&Frame::Rejected{id,error}).await?,
                        }
                    }
                    Request::Snapshot{id}=>{write(&mut writer,&Frame::State{state:Box::new(service.snapshot())}).await?;let _=id;}
                    Request::Cancel{operation}=>{
                        match service.cancel(owner,operation) {Ok(())=>write(&mut writer,&Frame::Cancelled{operation}).await?,Err(error)=>write(&mut writer,&Frame::Rejected{id:0,error}).await?}
                    }
                    Request::Hello{..}=>return Err("duplicate handshake".into()),
                }
            }
            result=async { if let Some((_,_,reply))=&mut active {reply.await} else {std::future::pending().await} }=>{
                let (id,_,_)=active.take().unwrap();
                write(&mut writer,&Frame::Completed{id,state:Box::new(service.snapshot()),result:result?}).await?;
            }
        }
    }
}

pub struct Client {
    writer: WriteHalf<NamedPipeClient>,
    frames: mpsc::Receiver<Result<Frame>>,
    reader_task: tokio::task::JoinHandle<()>,
    pub state: AgentState,
    next: u64,
}
impl Drop for Client {
    fn drop(&mut self) {
        self.reader_task.abort();
    }
}
impl Client {
    pub async fn connect(root: &Path) -> Result<Self> {
        let name = pipe_name(root)?;
        let mut pipe = ClientOptions::new().open(name)?;
        verify_peer(pipe.as_raw_handle(), false)?;
        write(
            &mut pipe,
            &Request::Hello {
                version: protocol::VERSION,
            },
        )
        .await?;
        let frame: Frame =
            timeout(Duration::from_secs(3), protocol::read_frame(&mut pipe)).await??;
        match frame {
            Frame::Welcome {
                version: protocol::VERSION,
                state,
            } => {
                let (mut reader, writer) = tokio::io::split(pipe);
                let (send, frames) = mpsc::channel(128);
                let reader_task = tokio::spawn(async move {
                    loop {
                        let frame = protocol::read_frame::<Frame>(&mut reader).await;
                        let failed = frame.is_err();
                        if send.send(frame).await.is_err() || failed {
                            break;
                        }
                    }
                });
                Ok(Self {
                    writer,
                    frames,
                    reader_task,
                    state: *state,
                    next: 1,
                })
            }
            _ => Err("agent handshake or version rejected".into()),
        }
    }
    fn accept_state(&mut self, state: AgentState) -> Result<()> {
        if state.instance != self.state.instance {
            return Err(protocol::unavailable(
                "Agent instance changed. Reconnect and reconcile before retrying.",
            ));
        }
        if state.sequence >= self.state.sequence {
            self.state = state;
        }
        Ok(())
    }
    pub async fn next_state(&mut self) -> Result<AgentState> {
        loop {
            let frame = self
                .frames
                .recv()
                .await
                .ok_or("agent event reader stopped")??;
            if let Frame::State { state } = frame {
                if state.sequence > self.state.sequence + 1 {
                    write(&mut self.writer, &Request::Snapshot { id: 0 }).await?;
                }
                self.accept_state(*state)?;
                return Ok(self.state.clone());
            }
        }
    }
    pub async fn execute(&mut self, command: Command, cancel: Arc<AtomicBool>) -> Result<Outcome> {
        self.execute_with_events(command, cancel, |_| {}).await
    }
    pub async fn execute_with_events(
        &mut self,
        command: Command,
        cancel: Arc<AtomicBool>,
        mut event: impl FnMut(&AgentState),
    ) -> Result<Outcome> {
        let persistent = matches!(command, Command::StartLive { .. });
        let id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or("request counter exhausted")?;
        write(&mut self.writer, &Request::Execute { id, command }).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let mut operation = None;
        let mut cancel_sent = false;
        let mut tick = tokio::time::interval(Duration::from_millis(25));
        loop {
            let frame = tokio::select! {
                result=self.frames.recv()=>result.ok_or("agent event reader stopped")??,
                _=tokio::time::sleep_until(deadline),if !persistent=>return Err("agent operation timed out".into()),
                _=tick.tick()=>{
                    if cancel.load(Ordering::Acquire) && !cancel_sent && let Some(operation)=operation {write(&mut self.writer,&Request::Cancel{operation}).await?;cancel_sent=true;}
                    continue;
                }
            };
            let state = &mut self.state;
            match frame {
                Frame::Accepted {
                    id: rid,
                    operation: op,
                } if rid == id => operation = Some(op),
                Frame::Completed {
                    id: rid,
                    state: new_state,
                    result,
                } if rid == id => {
                    if new_state.instance != state.instance {
                        return Err(protocol::unavailable("Agent restarted."));
                    }
                    *state = *new_state;
                    event(state);
                    return result.map_err(|e| Box::new(e) as crate::Error);
                }
                Frame::Rejected { id: rid, error } if rid == id || rid == 0 => {
                    return Err(Box::new(error));
                }
                Frame::State { state: new_state } => {
                    if new_state.instance != state.instance {
                        return Err(protocol::unavailable("Agent restarted."));
                    }
                    if new_state.sequence >= state.sequence {
                        *state = *new_state;
                        event(state);
                    }
                }
                _ => {}
            }
        }
    }
}

pub async fn connect_or_launch(root: &Path, executable: &Path) -> Result<Client> {
    std::fs::create_dir_all(root)?;
    if let Ok(client) = Client::connect(root).await {
        return Ok(client);
    }
    use std::os::windows::process::CommandExt;
    std::process::Command::new(executable)
        .arg("--data-dir")
        .arg(root)
        .arg("--idle-exit-ms")
        .arg("30000")
        .creation_flags(0x08000000)
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match Client::connect(root).await {
            Ok(client) => return Ok(client),
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Controller, Diagnostic};
    async fn connect(root: &Path) -> Client {
        for _ in 0..100 {
            if let Ok(client) = Client::connect(root).await {
                return client;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("test agent did not accept a local SDK connection");
    }
    #[tokio::test]
    async fn real_pipe_sdk_reconciles_two_clients_cancels_and_rejects_bad_versions() {
        let root = std::env::temp_dir().join(format!(
            "bricks-pipe-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let service = Service::new(Controller::open(root.clone()).unwrap()).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = stop.clone();
        let server_root = root.clone();
        let controller = service.clone();
        let server =
            tokio::spawn(async move { serve(&server_root, controller, server_stop, None).await });
        let mut a = connect(&root).await;
        let mut b = connect(&root).await;
        assert_eq!(a.state.instance, b.state.instance);
        let result = a
            .execute(
                Command::SaveDevice {
                    revision: 0,
                    id: None,
                    name: "SDK PC".into(),
                    ip: "100.101.102.103".into(),
                    port: "47990".into(),
                },
                Arc::new(AtomicBool::new(false)),
            )
            .await
            .unwrap();
        assert_eq!(result.snapshot.unwrap().revision, 1);
        while b.state.snapshot.revision < 1 {
            timeout(Duration::from_secs(3), b.next_state())
                .await
                .unwrap()
                .unwrap();
        }
        assert_eq!(b.state.snapshot.devices[0].name, "SDK PC");
        let stale = b
            .execute(
                Command::Settings {
                    revision: 0,
                    name: "Stale".into(),
                    port: "47990".into(),
                },
                Arc::new(AtomicBool::new(false)),
            )
            .await
            .unwrap_err();
        assert!(stale.to_string().contains("RevisionConflict"));
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            flag.store(true, Ordering::Release);
        });
        let cancelled = a
            .execute(
                Command::Check {
                    kind: Diagnostic::RecoveryNetwork,
                },
                cancel,
            )
            .await
            .unwrap_err();
        assert!(cancelled.to_string().contains("Cancelled"), "{cancelled}");
        let mut raw = ClientOptions::new()
            .open(pipe_name(&root).unwrap())
            .unwrap();
        protocol::write_frame(&mut raw, &Request::Hello { version: 99 })
            .await
            .unwrap();
        assert!(
            matches!(protocol::read_frame::<Frame>(&mut raw).await.unwrap(),Frame::Rejected{error:Failure{code,..},..} if code=="IncompatibleVersion")
        );
        drop(raw);
        let active = tokio::spawn(async move {
            a.execute(
                Command::Check {
                    kind: Diagnostic::RecoveryNetwork,
                },
                Arc::new(AtomicBool::new(false)),
            )
            .await
        });
        let until = tokio::time::Instant::now() + Duration::from_secs(2);
        while service.snapshot().active_operations.is_empty() {
            assert!(tokio::time::Instant::now() < until);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        drop(b);
        stop.store(true, Ordering::Release);
        server.await.unwrap().unwrap();
        assert!(active.await.unwrap().is_err());
        let until = tokio::time::Instant::now() + Duration::from_secs(3);
        while !service.snapshot().active_operations.is_empty() {
            assert!(
                tokio::time::Instant::now() < until,
                "aborted IPC task must cancel its owned operation"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        drop(service);
        // Wait for the storage worker to release its state lock and temporary root.
        for _ in 0..100 {
            if !root.join("agent-running.json").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
