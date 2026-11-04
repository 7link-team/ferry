import { useSyncExternalStore, type CSSProperties } from "react";
import { activitySnapshot, subscribeActivity } from "../api/progress";
import "./activity-progress.css";

export function ActivityProgress() {
  const activities = useSyncExternalStore(subscribeActivity, activitySnapshot);
  if (!activities.length) return null;
  return <div className="activity-progress" aria-hidden="true">
    {[...activities].sort((a, b) => Number(a.kind === "request") - Number(b.kind === "request")).map(activity =>
      <div key={activity.id} className="activity-progress-layer" data-kind={activity.kind} data-done={activity.done}
        style={{ "--activity-hue": activity.hueOffset, "--activity-duration": `${5 + activity.id % 7}s` } as CSSProperties} />)}
  </div>;
}
