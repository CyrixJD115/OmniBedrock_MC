// Deployment switch preflight (GET /instances/{id}/migration?to=…).
export interface MigrationCheck {
  id: string;
  level: 'ok' | 'info' | 'warning' | 'error';
  message: string;
}

export interface MigrationPlan {
  from: string;
  to: string;
  ready: boolean;
  checks: MigrationCheck[];
  steps: string[];
}
