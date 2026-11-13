import { api } from "../api";
import Icon from "../Icon";
import { overall, STAGE_TEXT, useQueue } from "../queue";

/** Everything waiting to be prepared in the background. */
export default function Queue() {
  const q = useQueue();
  const waiting = q.items.filter((i) => i.state === "waiting");
  const failed = q.items.filter((i) => i.state === "failed");
  let n = 0;

  return (
    <div className="page narrow">
      <div className="row head">
        <h1>Queue</h1>
        <div className="spacer" />
        {q.items.length > 0 && failed.length < q.items.length && (
          <button className="ghost small" onClick={() => api.queuePause(!q.paused)}>
            <Icon name={q.paused ? "play" : "pause"} size={15} /> {q.paused ? "Resume" : "Pause"}
          </button>
        )}
        {failed.length > 0 && (
          <button className="ghost small" onClick={() => api.queueClear(true)}>
            Clear failed
          </button>
        )}
        {waiting.length > 1 && (
          <button className="ghost small" onClick={() => api.queueClear(false)}>
            Clear all
          </button>
        )}
      </div>
      <p className="muted">
        Videos here are prepared one after another in the background while you watch something else, browse, or leave
        LingoReel open. The queue picks up where it left off after a restart.
        {q.paused && " Paused: nothing new starts until you resume. Pausing frees the graphics card straight away."}
      </p>

      {q.items.length === 0 ? (
        <div className="empty small-empty">
          <div className="empty-art">
            <Icon name="queue" size={34} />
          </div>
          <h2>Nothing waiting</h2>
          <p className="muted">
            In the Library, hover over a movie, series or episode and press <Icon name="queue-add" size={15} />, or use
            “Queue season” on a series, to have it ready before you sit down to watch.
          </p>
        </div>
      ) : (
        <div className="queue-list">
          {q.items.map((it) => {
            const pct = overall(it.stage, it.pct);
            const pos = it.state === "waiting" ? ++n : 0;
            return (
              <div key={it.key} className={"qrow " + it.state}>
                <span className="qnum">
                  {it.state === "working" ? <i className="spin" /> : it.state === "failed" ? "!" : pos}
                </span>
                <div className="qrow-main">
                  <div className="qrow-title" title={it.title}>
                    {it.title}
                  </div>
                  {it.state === "working" && (
                    <>
                      <div className="qrow-sub">
                        {STAGE_TEXT[it.stage] || "Starting"} · {Math.round(pct)}%
                      </div>
                      <div className="thin-bar">
                        <div style={{ width: pct + "%" }} />
                      </div>
                    </>
                  )}
                  {it.state === "waiting" && <div className="qrow-sub">{q.paused ? "Paused" : "Waiting"}</div>}
                  {it.state === "failed" && <div className="qrow-sub err">{it.error}</div>}
                </div>
                {it.state === "failed" && (
                  <button className="icon-btn small" title="Try again" onClick={() => api.queueRetry(it.key)}>
                    <Icon name="retry" size={16} />
                  </button>
                )}
                <button
                  className="icon-btn small"
                  title={it.state === "working" ? "Stop and remove from the queue" : "Remove from the queue"}
                  onClick={() => api.queueRemove(it.key)}
                >
                  <Icon name="close" size={16} />
                </button>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
