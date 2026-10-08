export type TypographyGeometrySource = 'width' | 'font' | 'line-height' | 'zoom';

export interface TypographyInteractionSnapshot {
  phase: 'idle' | 'preview' | 'settling';
  generation: number;
  geometryRevision: number;
  geometrySource?: TypographyGeometrySource;
  anchor?: { left: number; top: number };
}

interface InteractionHold {
  source: TypographyGeometrySource;
  explicit: boolean;
}

interface InteractionBurst {
  hold: InteractionHold;
  timer: ReturnType<typeof setTimeout>;
}

interface Subscriber {
  listener(snapshot: TypographyInteractionSnapshot): void;
  geometryParticipant: boolean | (() => boolean);
  beforeGeometryChange?: (nextSnapshot: TypographyInteractionSnapshot) => (() => void) | void;
}

const QUIET_PERIOD_MS = 150;
const holds = new Set<InteractionHold>();
const bursts = new Map<TypographyGeometrySource, InteractionBurst>();
const subscribers = new Map<number, Subscriber>();
const pending = new Set<number>();
let snapshot: TypographyInteractionSnapshot = {
  phase: 'idle',
  generation: 0,
  geometryRevision: 0,
};
let nextSubscriberId = 0;
let notifying = 0;

function isGeometryParticipant(subscriber: Subscriber): boolean {
  return typeof subscriber.geometryParticipant === 'function'
    ? subscriber.geometryParticipant()
    : subscriber.geometryParticipant;
}

export function getTypographyInteraction(): TypographyInteractionSnapshot {
  return {
    ...snapshot,
    anchor: snapshot.anchor ? { ...snapshot.anchor } : undefined,
  };
}

/** Controls always read the geometry currently displayed in this window. */
export function getTypographyGeometryValue(name: string): string {
  const root = document.documentElement;
  return root.style.getPropertyValue(name) || getComputedStyle(root).getPropertyValue(name);
}

function finishSettlement() {
  if (notifying || snapshot.phase !== 'settling' || pending.size) return;
  snapshot = {
    phase: 'idle',
    generation: snapshot.generation,
    geometryRevision: snapshot.geometryRevision,
    geometrySource: snapshot.geometrySource,
  };
  notifySubscribers();
}

function notifySubscribers() {
  notifying++;
  try {
    const current = getTypographyInteraction();
    for (const [id, subscriber] of [...subscribers]) {
      if (
        snapshot.generation !== current.generation ||
        snapshot.phase !== current.phase ||
        snapshot.geometryRevision !== current.geometryRevision
      )
        break;
      if (subscribers.has(id)) subscriber.listener(current);
    }
  } finally {
    notifying--;
    finishSettlement();
  }
}

function releaseHold(hold: InteractionHold) {
  if (!holds.delete(hold) || holds.size) return;
  snapshot = { ...snapshot, phase: 'settling' };
  pending.clear();
  for (const id of subscribers.keys()) pending.add(id);
  notifySubscribers();
}

/** A control or animation holds the interactive session until its final change. */
export function beginTypographyInteraction(
  source: TypographyGeometrySource,
  anchor?: TypographyInteractionSnapshot['anchor'],
): () => void {
  const hold: InteractionHold = { source, explicit: true };
  holds.add(hold);
  const burst = bursts.get(source);
  if (burst) {
    clearTimeout(burst.timer);
    bursts.delete(source);
    holds.delete(burst.hold);
  }
  if (snapshot.phase !== 'preview') {
    pending.clear();
    snapshot = {
      ...snapshot,
      phase: 'preview',
      generation: snapshot.generation + 1,
      anchor,
    };
    notifySubscribers();
  } else if (anchor) {
    snapshot = { ...snapshot, anchor };
  }
  return () => releaseHold(hold);
}

function holdGeometrySource(source: TypographyGeometrySource) {
  if ([...holds].some((hold) => hold.source === source && hold.explicit)) return;
  const previous = bursts.get(source);
  if (previous) clearTimeout(previous.timer);
  const hold = previous?.hold ?? { source, explicit: false };
  holds.add(hold);
  const burst: InteractionBurst = {
    hold,
    timer: setTimeout(() => {
      if (bursts.get(source) !== burst) return;
      bursts.delete(source);
      releaseHold(hold);
    }, QUIET_PERIOD_MS),
  };
  bursts.set(source, burst);
}

function nextGeometrySnapshot(
  source: TypographyGeometrySource,
  anchor?: TypographyInteractionSnapshot['anchor'],
): TypographyInteractionSnapshot {
  return {
    phase: 'preview',
    generation: snapshot.generation + 1,
    geometryRevision: snapshot.geometryRevision + 1,
    geometrySource: source,
    anchor,
  };
}

/** External geometry changes share the same quiet-period settlement as controls. */
export function notifyTypographyGeometryChange(
  source: TypographyGeometrySource,
  anchor?: TypographyInteractionSnapshot['anchor'],
) {
  holdGeometrySource(source);
  pending.clear();
  snapshot = nextGeometrySnapshot(source, anchor);
  notifySubscribers();
}

/** CSS follows input immediately; participants only preserve their viewport anchors. */
export function requestTypographyGeometryChange(
  name: string,
  value: string,
  source: TypographyGeometrySource,
  anchor?: TypographyInteractionSnapshot['anchor'],
) {
  if (getTypographyGeometryValue(name).trim() === value.trim()) return;
  const nextSnapshot = nextGeometrySnapshot(source, anchor);
  const afterChange: { id: number; callback: () => void }[] = [];
  for (const [id, subscriber] of [...subscribers]) {
    if (!subscribers.has(id) || !isGeometryParticipant(subscriber)) continue;
    const callback = subscriber.beforeGeometryChange?.({
      ...nextSnapshot,
      anchor: nextSnapshot.anchor ? { ...nextSnapshot.anchor } : undefined,
    });
    if (callback) afterChange.push({ id, callback });
  }
  document.documentElement.style.setProperty(name, value);
  holdGeometrySource(source);
  pending.clear();
  snapshot = nextSnapshot;
  try {
    for (const { id, callback } of afterChange) if (subscribers.has(id)) callback();
  } finally {
    notifySubscribers();
  }
}

/** Every editor settles independently; geometry participants never delay CSS. */
export function subscribeTypographyInteraction(
  listener: (snapshot: TypographyInteractionSnapshot) => void,
  options: {
    geometryParticipant?: boolean | (() => boolean);
    beforeGeometryChange?: (nextSnapshot: TypographyInteractionSnapshot) => (() => void) | void;
  } = {},
) {
  const id = ++nextSubscriberId;
  const subscriber: Subscriber = {
    listener,
    geometryParticipant: options.geometryParticipant ?? false,
    beforeGeometryChange: options.beforeGeometryChange,
  };
  subscribers.set(id, subscriber);
  if (snapshot.phase === 'settling') pending.add(id);
  notifying++;
  try {
    listener(getTypographyInteraction());
  } finally {
    notifying--;
    finishSettlement();
  }
  return {
    complete(generation: number) {
      if (snapshot.phase !== 'settling' || snapshot.generation !== generation) return;
      pending.delete(id);
      finishSettlement();
    },
    destroy() {
      subscribers.delete(id);
      pending.delete(id);
      finishSettlement();
    },
  };
}
