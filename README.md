<p align="center">
  <img src="assets/OmniBedrock_Icon.png" alt="OmniBedrock" width="200">
</p>

# OmniBedrock

A modern, powerful web-based Minecraft Bedrock server control panel. Migrated from PySide6 to a full-stack FastAPI + SvelteKit architecture.

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
├── backend/          # FastAPI + Python (server logic, API, WebSockets)
│   ├── app/
│   │   ├── core/         # Config, security, logging, DI
│   │   ├── routers/      # REST API endpoints
│   │   ├── services/     # Business logic
│   │   ├── managers/     # Server state, backup scheduler, performance
│   │   ├── models/       # Pydantic data models
│   │   ├── schemas/      # Request/response schemas
│   │   ├── websocket/    # WebSocket handlers
│   │   └── utils/        # Helpers
│   └── requirements.txt
├── frontend/         # SvelteKit + TypeScript + TailwindCSS
│   └── src/
│       ├── routes/       # Page components (dashboard, console, etc.)
│       ├── lib/          # API client, WebSocket, utilities
│       ├── components/   # Reusable UI components
│       ├── stores/       # Svelte stores
│       └── types/        # TypeScript interfaces
├── docker-compose.yml
└── .env.example
```

## Quick Start

### Prerequisites

- Python 3.11+
- Node.js 20+
- uv (Python package manager) or pip
- A Minecraft Bedrock server setup with Endstone

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

# Start the backend
uv run uvicorn backend.app.main:app --reload --port 8000
```

The backend prints an auth token on first startup. Copy it — you'll need it to log into the frontend.

### Frontend

```bash
cd frontend
npm install
npm run dev
```

Open http://localhost:5173 in your browser and enter the auth token from the backend.

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/v1/server/status` | Server status |
| POST | `/api/v1/server/action` | Start/stop/restart/kill |
| POST | `/api/v1/console/command` | Send console command |
| WS | `/api/v1/console/ws` | Console WebSocket stream |
| GET | `/api/v1/properties/` | Server properties |
| PUT | `/api/v1/properties/{key}` | Update property |
| GET/POST | `/api/v1/backups/` | List/create backups |
| GET | `/api/v1/addons/` | List behavior/resource packs |
| GET/POST | `/api/v1/players/` | List players / send action |
| GET | `/api/v1/worlds/` | List worlds |
| WS | `/api/v1/performance/ws` | Real-time metrics |

## Migration from PySide6

The original PySide6 desktop app has been fully refactored:

- **All UI** → SvelteKit frontend (dark theme, responsive)
- **Server management** → `backend/app/services/server_manager.py`
- **Console I/O** → `backend/app/services/console_stream.py`
- **Properties editor** → `backend/app/services/properties_service.py`
- **Backup engine** → `backend/app/services/backup_service.py`
- **Addon management** → `backend/app/services/addon_service.py`
- **Configuration** → `backend/app/core/config.py` (pydantic-settings)
- **Auth** → Simple token-based (see `backend/app/core/security.py`)

## License

MIT
