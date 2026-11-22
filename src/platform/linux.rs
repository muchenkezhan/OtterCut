//! Linux capture through the desktop portal and PipeWire.

use std::cell::Cell;
use std::future::Future;
use std::os::fd::OwnedFd;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
use ashpd::desktop::{PersistMode, ResponseError, Session};
use async_io::Timer;
use futures_lite::future;
use futures_lite::stream::StreamExt as _;

use super::{RecordingRequest, SavedRecording};

mod pipeline;

const PORTAL_SETUP_TIMEOUT: Duration = Duration::from_secs(10);
const PORTAL_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const STOP_POLL: Duration = Duration::from_millis(75);

pub fn record(
    request: RecordingRequest,
    stopped: &AtomicBool,
    on_started: impl FnOnce(),
) -> Result<Option<SavedRecording>> {
    if stopped.load(Ordering::Relaxed) {
        return Ok(None);
    }

    future::block_on(async {
        let Some(portal) = until_stopped(
            stopped,
            with_timeout(PORTAL_SETUP_TIMEOUT, Screencast::new()),
        )
        .await
        else {
            return Ok(None);
        };
        let portal = portal
            .and_then(|result| result.map_err(Into::into))
            .context("无法连接桌面录屏服务，请检查 XDG Desktop Portal")?;
        if stopped.load(Ordering::Relaxed) {
            return Ok(None);
        }

        let session = with_timeout(PORTAL_SETUP_TIMEOUT, portal.create_session())
            .await
            .context("创建录屏会话超时")?
            .context("无法创建录屏会话")?;
        let result = record_session(&portal, &session, request, stopped, on_started).await;
        // Close the portal even when selection is cancelled or the encoder fails.
        let _ = with_timeout(PORTAL_CLOSE_TIMEOUT, session.close()).await;
        result
    })
}

async fn record_session(
    portal: &Screencast<'_>,
    session: &Session<'_, Screencast<'_>>,
    request: RecordingRequest,
    stopped: &AtomicBool,
    on_started: impl FnOnce(),
) -> Result<Option<SavedRecording>> {
    let mut closed_signal = pin!(
        with_timeout(PORTAL_SETUP_TIMEOUT, session.receive_closed())
            .await?
            .context("无法监听录屏会话状态")?
    );
    let Some(source) = until_stopped(stopped, select_source(portal, session)).await else {
        return Ok(None);
    };
    let (remote, node) = match source {
        Ok(source) => source,
        Err(error) => {
            if matches!(
                error.downcast_ref::<ashpd::Error>(),
                Some(ashpd::Error::Response(ResponseError::Cancelled))
            ) {
                return Ok(None);
            }
            return Err(error);
        }
    };

    let portal_closed = Cell::new(false);
    let mut saved = future::race(
        pipeline::record(
            remote,
            node,
            request,
            || stopped.load(Ordering::Relaxed) || portal_closed.get(),
            on_started,
        ),
        async {
            // Older PipeWire plugins can flush on disconnect without sending EOS.
            let _ = closed_signal.next().await;
            portal_closed.set(true);
            future::pending().await
        },
    )
    .await?;
    if portal_closed.get()
        && let Some(saved) = &mut saved
    {
        saved.warning = Some("系统录屏会话已结束".into());
    }
    Ok(saved)
}

async fn select_source(
    portal: &Screencast<'_>,
    session: &Session<'_, Screencast<'_>>,
) -> Result<(OwnedFd, u32)> {
    let supported = with_timeout(PORTAL_SETUP_TIMEOUT, portal.available_source_types())
        .await?
        .context("无法查询桌面录屏来源")?;
    let requested = supported & (SourceType::Monitor | SourceType::Window);
    ensure!(!requested.is_empty(), "当前桌面不支持屏幕或窗口录制");

    let cursor_mode =
        match with_timeout(PORTAL_SETUP_TIMEOUT, portal.available_cursor_modes()).await? {
            Ok(modes) if modes.contains(CursorMode::Embedded) => CursorMode::Embedded,
            _ => CursorMode::Hidden,
        };
    with_timeout(
        PORTAL_SETUP_TIMEOUT,
        portal.select_sources(
            session,
            cursor_mode,
            requested,
            false,
            None,
            PersistMode::DoNot,
        ),
    )
    .await?
    .context("无法设置录屏来源")?
    .response()
    .context("桌面拒绝设置录屏来源")?;

    let streams = portal
        .start(session, None)
        .await
        .context("无法打开系统录屏来源选择器")?
        .response()
        .context("没有获得录屏授权")?;
    let stream = streams.streams().first().context("系统未返回录屏画面")?;
    let remote = with_timeout(PORTAL_SETUP_TIMEOUT, portal.open_pipe_wire_remote(session))
        .await?
        .context("无法连接已获授权的 PipeWire 画面")?;
    Ok((remote, stream.pipe_wire_node_id()))
}

async fn with_timeout<T>(timeout: Duration, operation: impl Future<Output = T>) -> Result<T> {
    future::race(async { Ok(operation.await) }, async {
        Timer::after(timeout).await;
        bail!("桌面录屏服务没有及时响应");
    })
    .await
}

async fn until_stopped<T>(stopped: &AtomicBool, operation: impl Future<Output = T>) -> Option<T> {
    if stopped.load(Ordering::Relaxed) {
        return None;
    }
    future::race(async { Some(operation.await) }, async {
        while !stopped.load(Ordering::Relaxed) {
            Timer::after(STOP_POLL).await;
        }
        None
    })
    .await
}
