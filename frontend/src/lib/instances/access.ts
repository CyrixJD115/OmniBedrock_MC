/**
 * Instance access helpers: role-catalog lookup shared by the instances page
 * (permission evaluation) and the instance card (shared-with display).
 */
import type { RoleInfo } from '$types/index';

/** Look up a role in the catalog by name. */
export function roleInfo(catalog: RoleInfo[], name: string): RoleInfo | undefined {
  return catalog.find((r) => r.name === name);
}
