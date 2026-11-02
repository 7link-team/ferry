export interface Credential {
  id: string;
  name: string;
  scope: string;
  kind: string;
  username: string;
  description: string;
  /** The server reports only whether a secret is set, never its value. */
  has_secret: boolean;
  /** Write-only: sent when creating or rotating, never returned by any read. */
  secret?: string;
  version: number;
}
