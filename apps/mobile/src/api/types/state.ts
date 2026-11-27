import type { AgentRow } from "./agents";
import type { Approval } from "./approvals";
import type { AreaTree } from "./areas";
import type { AuthInfo } from "./auth";
import type { BrowserInfo, ChromePairing, ChromeProfile } from "./browser";
import type { ChatMsg, Live } from "./chat";
import type { Day, Instant } from "./common";
import type { Author } from "../wire";
import type { AgentNow, RunnerSession, ExternalThread } from "./sessions";
import type { Issue, NeedKind, TaskArtifact, TaskLive } from "./tasks";
import type { ThemeChoice } from "./theme";

/** A room: Main ("Emberhut", id 1) or one per project, each its own conversation. */
export interface RoomRow {
  id: number;
  name: string;
  project?: number | null;
  goal?: number | null;
  /** Emberhut's answers you haven't seen. */
  unread: number;
  /** Tasks in it waiting on you. */
  needs: number;
  /** The first one's action line. */
  ask?: string | null;
  /** The task behind `ask`. */
  askId?: number | null;
  busy: boolean;
  /** Its status line while Emberhut works in it. */
  doing?: string | null;
  last?: string | null;
  lastAt?: Instant | null;
  lastMine?: boolean | null;
  active: boolean;
  /** Its project's status: active, paused, archived, done. */
  status?: string | null;
  pausedUntil?: Day | null;
}

/** A project. priority: 0 none, 1 urgent … 4 low; health comes from its hard deadlines. */
export interface Project {
  id: number;
  name: string;
  goal?: number | null;
  priority: number;
  /** deliverable or ongoing. */
  kind: string;
  deadline?: Day | null;
  hard: boolean;
  status: "active" | "paused" | "archived" | "done";
  /** Paused: the day it comes back by itself; null until you resume it. */
  pausedUntil?: Day | null;
  open: number;
  doneWeek: number;
  health?: "on_track" | "at_risk" | "late" | null;
  /** The nearest deadline of its open tasks. */
  next?: Day | null;
  hoursLeft: number;
}

/** One piece of work in flight: a task (worked by Emberhut, a thread, or waiting on you) or a thread with no task. */
export interface WorkRow {
  id: string;
  kind: "task" | "thread";
  task?: number | null;
  machine?: string | null;
  thread?: string | null;
  title: string;
  pid?: number | null;
  area?: number | null;
  who: string;
  /** waiting (needs you), running, stuck, limit, stopped, queued, stalled, scheduled. */
  state: string;
  progress?: string | null;
  line?: string | null;
  at?: Instant | null;
  eta?: string | null;
  ask?: string | null;
  last?: string | null;
  /** Needs you: what you should do, its answers, and whether a draft comes with it. */
  action?: string | null;
  options?: string[] | null;
  draft?: boolean | null;
  need?: NeedKind | null;
  live?: TaskLive | null;
  /** Its newest picture or video, small. */
  thumb?: string | null;
  /** An X post to reply to. */
  reply?: TaskArtifact | null;
  asked?: Instant | null;
  /** When Emberhut last checked it still holds. */
  checked?: Instant | null;
  /** Its decide-by or its deadline, the earlier. */
  decideBy?: Day | null;
  /** A deadline someone holds you to. */
  hardBy?: boolean | null;
  /** Its project is paused: it waits with Later, counted nowhere. */
  paused?: boolean | null;
}

export interface Goal {
  id: number;
  title: string;
  why: string;
  /** 1 matters most. */
  rank: number;
  status?: string | null;
  /** The label under its ring. */
  short?: string | null;
  week: Record<Author, number>;
}

/** A small task under a goal, yours (who is OWNER) or Emberhut's. */
export interface Todo {
  id: number;
  goal?: number | null;
  text: string;
  who: string;
  today: boolean;
  done?: Instant | null;
}

/** The weekly report. */
export interface Report {
  concern?: { title: string; why: string } | null;
  /** Tasks finished in each of the last 8 weeks, this one last. */
  weeks: number[];
  weekStarts?: Day[] | null;
  since?: Day | null;
  goals: {
    id: number;
    title: string;
    short?: string | null;
    done: number;
    projects: { id: number; name: string; doneWeek: number; open: number; health?: string | null }[];
  }[];
  done: { id: number; title: string; owner: string; project?: string | null }[];
}

/** A source Emberhut reads (Health, Slack, a coding app…). */
export interface Source {
  id: string;
  name: string;
  detail: string;
  at?: Instant | null;
  error?: string | null;
  state: "ok" | "stale" | "down";
}

/** The Claude account Emberhut's turns run on and its quota (0…1). */
export interface Account {
  /** The account's id in Emberhut's rotation. */
  id?: string | null;
  email?: string | null;
  plan?: string | null;
  loggedIn?: boolean | null;
  used5h?: number | null;
  resets5h?: Instant | null;
  used7d?: number | null;
  resets7d?: Instant | null;
  limitedUntil?: Instant | null;
  /** When the quota was last read. */
  at?: Instant | null;
}

/** A Claude account Emberhut and its runner rotate through. No tokens, ever. */
export interface ClaudeAccount {
  id: string;
  name: string;
  email?: string | null;
  loggedIn?: boolean | null;
  used5h?: number | null;
  used7d?: number | null;
  limitedUntil?: Instant | null;
  /** The runner machine it's kept on (an id of GET /api/machines); none: not tied to one. */
  machine?: string | null;
  /** any, or the one kind of work it is kept for. */
  scope?: string | null;
  enabled?: boolean | null;
}

/** The account Emberhut itself runs on: `on` now, `pin` your choice (null: Auto). */
export interface AssistantAccount {
  on?: string | null;
  pin?: string | null;
  mode?: string | null;
  /** The pinned one can't take work now, so Emberhut runs elsewhere until it can. */
  pinOut?: boolean | null;
}

/** One account on the Usage card, use in % (0…100). `best` has the most room. */
export interface UsageRow {
  id: string;
  name: string;
  scope?: string | null;
  loggedIn?: boolean | null;
  used5h?: number | null;
  resets5h?: Instant | null;
  used7d?: number | null;
  resets7d?: Instant | null;
  limitedUntil?: Instant | null;
  checked?: Instant | null;
  places?: string[] | null;
  best?: boolean | null;
}

export interface HealthNow {
  date: Day;
  steps?: number | null;
  exerciseMin?: number | null;
  moveMin?: number | null;
  stepsTarget: number;
  at?: Instant | null;
  project?: number | null;
  tasks: { id: number; target: number }[];
}

export interface Checkin {
  at: Instant;
  why: string;
}

/** A cheap watcher keeping an eye on something; it wakes Emberhut only with news. */
export interface Watch {
  id: number;
  title?: string | null;
  what: string;
  every: number;
  until: Instant;
  nextAt: Instant;
  /** What it saw last. */
  note: string;
  noteAt?: Instant | null;
}

/** Work Emberhut handed off to a thread; `thread` is its latest status. */
export interface Handed {
  id: string;
  machine: string;
  why: string;
  at: Instant;
  thread?: { status: string; title: string; project: string } | null;
}

/** Emberhut's own background agents and commands. */
export interface BackgroundJob {
  id: string;
  description: string;
  type: string;
  status: string;
  startedAt: Instant;
  endedAt?: Instant | null;
  summary?: string | null;
}

/** GET /api/state: everything the app shows, for one room. */
export interface EmberhutState {
  /** Emberhut is on (not paused). */
  on: boolean;
  busy: boolean;
  live?: Live | null;
  /** The server can push to this kind of device. */
  push: boolean;
  unread: boolean;
  /** Events Emberhut has not looked at yet. */
  pending: number;
  checkins: Checkin[];
  tasks?: BackgroundJob[] | null;
  browser?: BrowserInfo | null;
  recording?: { id: string; title: string; at: Instant } | null;
  health?: HealthNow | null;
  auth?: AuthInfo | null;
  followed?: Handed[] | null;
  watches?: Watch[] | null;
  did: { time: string; at: Instant; text: string }[];
  /** The newest 60, oldest first. */
  chat: ChatMsg[];
  /** Older messages than these exist. */
  earlier: boolean;
  account?: Account | null;
  healthAt?: Instant | null;
  goals?: Goal[] | null;
  todos?: Todo[] | null;
  /** Every task: open ones and this week's done ones. */
  issues?: Issue[] | null;
  projects?: Project[] | null;
  areas?: AreaTree[] | null;
  /** Today's task ids, ranked by the server. */
  today?: number[] | null;
  report?: Report | null;
  /** Everything in flight, waiting on you first. */
  working?: WorkRow[] | null;
  /** The server has an OpenAI key: calls and dictation work. */
  voice?: boolean | null;
  awareness?: Source[] | null;
  /** The room this state is for. */
  room: number;
  /** Main first, then by last activity. */
  rooms?: RoomRow[] | null;
  /** This room's sessions on Emberhut's runner (Main: all), newest first. */
  sessions?: RunnerSession[] | null;
  /** Your own sessions, every room's, newest first. */
  mySessions?: RunnerSession[] | null;
  agentsNow?: AgentNow[] | null;
  t3Running?: ExternalThread[] | null;
  accounts?: ClaudeAccount[] | null;
  usage?: UsageRow[] | null;
  /** The account the assistant runs on, Auto or pinned. */
  assistantAccount?: AssistantAccount | null;
  chrome?: ChromeProfile[] | null;
  chromePairing?: ChromePairing[] | null;
  chromeLatest?: string | null;
  /** What the app hides for now. */
  ui?: { hideGoals?: boolean | null; hideProjects?: boolean | null } | null;
  /** The newest build per platform. */
  builds?: { ios?: number | null; mac?: number | null } | null;
  /** Your theme choice; a server on older code has none. */
  theme?: ThemeChoice | null;
  /** Every agent, the default agent ("main") first. Left out by a server without agents. */
  agents?: AgentRow[] | null;
  /** The approval requests waiting on an answer, yours or an approver's, newest first. Left out when there are none. */
  approvals?: Approval[] | null;
}
