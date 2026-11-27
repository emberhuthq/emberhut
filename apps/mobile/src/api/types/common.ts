/** The server's plain answer to an action. Its 500 handler sends `{ error }` without `ok`. */
export interface Reply {
  ok?: boolean;
  error?: string;
}

/** A calendar day as the server writes it: "2026-10-15". */
export type Day = string;

/** An ISO 8601 date and time. */
export type Instant = string;
