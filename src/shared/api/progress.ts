/** Activity lifetimes, not invented download percentages. URLs stay out of the DOM. */
export interface Activity {
  id: number;
  kind: "route" | "request";
  hueOffset: number;
  done: boolean;
}

let sequence = 0;
let activities: readonly Activity[] = [];
const listeners = new Set<() => void>();
const sessionSeed = Math.floor(Math.random() * 0xffff);
const emit = () => listeners.forEach(listener => listener());

export const activitySnapshot = () => activities;
export const subscribeActivity = (listener: () => void) => {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
};

function hueOffset(url: string) {
  // Stable for the same endpoint during this session; omit query tokens/filters.
  const path = new URL(url, location.origin).pathname;
  let hash = sessionSeed;
  for (const character of path) hash = Math.imul(hash, 31) + character.charCodeAt(0) | 0;
  return 75 + (hash >>> 0) % 211;
}

export function beginActivity(kind: Activity["kind"], url: string) {
  const id = ++sequence;
  activities = [...activities, { id, kind, hueOffset: hueOffset(url), done: false }];
  emit();
  let finished = false;
  return () => {
    if (finished) return;
    finished = true;
    activities = activities.map(activity => activity.id === id ? { ...activity, done: true } : activity);
    emit();
    setTimeout(() => {
      activities = activities.filter(activity => activity.id !== id);
      emit();
    }, 180);
  };
}

let route: { url: string; finish: () => void } | undefined;
let committedUrl: string | undefined;
export function beginRoute(url: string) {
  if (route?.url === url) return;
  route?.finish();
  if (url === committedUrl) { route = undefined; return; }
  route = { url, finish: beginActivity("route", url) };
}

/** The route commits independently of the requests made by its page. */
export function commitRoute(url: string) {
  committedUrl = url;
  const committed = route;
  if (!committed || committed.url !== url) return;
  requestAnimationFrame(() => requestAnimationFrame(() => {
    if (route !== committed) return;
    committed.finish();
    route = undefined;
  }));
}
