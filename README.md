<h1 align="center">OmniBedrock</h1>
<p align="center">
  <img src="assets/OmniBedrock_Icon.png" alt="OmniBedrock" width="250">
</p>

A modern, powerful web-based Minecraft Bedrock server control panel.

## Features

- **Server Management** — Start, stop, restart, kill your Bedrock server (powered by Endstone)
- **Live Console** — Real-time interactive terminal with command history, filtering, syntax highlighting
- **Player Management** — List, kick, ban, pardon, op/deop via direct stdin commands
- **Server Properties** — Visual editor with validation + raw mode, backup catalog
- **World Backups** — Manual and scheduled backups with pre/post commands, download/restore
- **Addon Organizer** — Browse behavior/resource packs, view/edit manifests, manage UUIDs
- **World Browser** — List worlds, view size/details, explore contents
- **Configuration Files** — Browse and edit INI files in-browser
- **Performance Dashboard** — Real-time CPU, memory, TPS metrics via WebSocket
- **Application Settings** — View and manage configuration
- **Dark Theme** — Premium dark UI with neon accents, Minecraft-inspired aesthetic

## Architecture

```
OmniBedrock_MC/
├── assets/           # Branding assets (logo, etc.)
├── backend/          # FastAPI + Python (server logic, API, WebSockets)
│   ├── app/
│   │   ├── core/         # Config, security, logging, auth, DI
│   │   ├── routers/      # REST API endpoints
│   │   ├── services/     # Business logic
│   │   ├── managers/     # Server state, backup scheduler, performance
│   │   ├── models/       # Data models
│   │   ├── schemas/      # Request/response schemas
│   │   ├── websocket/    # WebSocket handlers
│   │   └── utils/        # Helpers
│   └── ini/              # Runtime data (users, config)
├── frontend/         # SvelteKit + TypeScript + TailwindCSS
│   └── src/
│       ├── routes/       # Page components (dashboard, console, etc.)
│       ├── lib/          # API client, WebSocket, utilities, UI components
│       ├── stores/       # Svelte stores
│       └── types/        # TypeScript interfaces
├── start.py          # Dual launcher (backend + frontend)
├── pyproject.toml    # Python project config
├── uv.lock           # Locked dependencies
└── .env.example
```

## Quick Start

### Prerequisites

- Python 3.11+
- Node.js 20+
- [uv](https://docs.astral.sh/uv/) (Python package manager)
- A Minecraft Bedrock server setup with [Endstone](https://endstone.dev/)

### Backend

```bash
# Install uv if not installed
curl -LsSf https://astral.sh/uv/install.sh | sh

# Clone and enter the project
cd OmniBedrock_MC

# Install dependencies
uv sync

# Copy env config
cp .env.example .env

# Start
uv run uvicorn backend.app.main:app --port 17754
```

On first startup, the backend creates a default admin account and prints the password to the console. Use it to log in at the frontend.

### Frontend

```bash
cd frontend
npm install
npm run dev
```

Open http://localhost:17755 and log in.

### Quick Start (both at once)

```bash
uv run python start.py
```

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/v1/auth/login` | Login (returns JWT) |
| GET | `/api/v1/auth/me` | Current user |
| GET/POST | `/api/v1/auth/users` | List / create users |
| PUT/DEL | `/api/v1/auth/users/{username}` | Update / delete user |
| GET | `/api/v1/server/status` | Server status |
| POST | `/api/v1/server/action` | Start/stop/restart/kill |
| POST | `/api/v1/console/command` | Send console command |
| WS | `/api/v1/console/ws` | Console WebSocket stream |
| GET | `/api/v1/properties/` | Server properties |
| PUT | `/api/v1/properties/{key}` | Update property |
| GET/POST | `/api/v1/backups/` | List / create backups |
| GET | `/api/v1/addons/` | List behavior/resource packs |
| GET/POST | `/api/v1/players/` | List / manage players |
| GET | `/api/v1/worlds/` | List worlds |
| GET | `/api/v1/audit` | Audit log (filterable) |
| GET/PUT | `/api/v1/settings/server` | Auto-restart & grace period settings |
| WS | `/api/v1/performance/ws` | Real-time CPU, RAM, TPS metrics |
| WS | `/api/v1/logs/ws` | Backend log stream |

## Migration from PySide6

The original PySide6 desktop app has been fully refactored:

- **All UI** → SvelteKit frontend (dark theme, responsive)
- **Server management** → `backend/app/services/server_manager.py`
- **Console I/O** → `backend/app/services/console_stream.py`
- **Properties editor** → `backend/app/services/properties_service.py`
- **Backup engine** → `backend/app/services/backup_service.py`
- **Addon management** → `backend/app/services/addon_service.py`
- **Configuration** → `backend/app/core/config.py` (pydantic-settings)
- **Auth** → JWT multi-user with role-based access (see `backend/app/core/auth.py`)

## License

MIT
