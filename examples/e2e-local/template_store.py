"""
Template Store — project scaffolds for common tech stacks.

Each template generates a working project skeleton with:
- Project structure (src/, tests/, configs)
- Dependencies (pyproject.toml, package.json, Cargo.toml)
- CI pipeline (.github/workflows/ci.yml)
- Pre-commit hooks (.pre-commit-config.yaml)
- Docker setup (Dockerfile, docker-compose.yml)
- Test skeleton (conftest.py, smoke tests)

Templates are matched to tasks by the HR Agent based on detected skills.
"""
import json
import os
from pathlib import Path


TEMPLATES = {
    # ── Python / FastAPI ──────────────────────────────────────────────
    "fastapi": {
        "name": "FastAPI Application",
        "match_skills": ["rest-api-development", "python-development"],
        "match_keywords": ["fastapi", "api", "endpoint", "rest"],
        "files": {
            "pyproject.toml": """[project]
name = "app"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = [
    "fastapi>=0.115",
    "uvicorn[standard]>=0.30",
    "pydantic>=2.0",
    "pydantic-settings>=2.0",
]

[project.optional-dependencies]
dev = ["pytest>=8.0", "httpx>=0.27", "ruff>=0.4"]

[tool.pytest.ini_options]
testpaths = ["tests"]
asyncio_mode = "auto"

[tool.ruff]
target-version = "py311"
line-length = 100
""",
            "src/__init__.py": '"""Application package."""\n',
            "src/main.py": """\"\"\"FastAPI application entry point.\"\"\"
from fastapi import FastAPI

app = FastAPI(title="App", version="0.1.0")


@app.get("/health")
async def health() -> dict:
    return {"status": "ok"}
""",
            "src/config.py": """\"\"\"Application configuration from environment.\"\"\"
from pydantic_settings import BaseSettings


class Settings(BaseSettings):
    app_name: str = "app"
    debug: bool = False
    database_url: str = ""

    class Config:
        env_file = ".env"


settings = Settings()
""",
            "tests/__init__.py": "",
            "tests/conftest.py": """import pytest
from fastapi.testclient import TestClient
from src.main import app


@pytest.fixture
def client():
    return TestClient(app)
""",
            "tests/test_health.py": """def test_health(client):
    response = client.get("/health")
    assert response.status_code == 200
    assert response.json()["status"] == "ok"
""",
            ".github/workflows/ci.yml": """name: CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with: { python-version: "3.11" }
      - run: pip install -e ".[dev]"
      - run: ruff check src/ tests/
      - run: pytest -v
""",
            ".pre-commit-config.yaml": """repos:
  - repo: https://github.com/astral-sh/ruff-pre-commit
    rev: v0.4.0
    hooks:
      - id: ruff
        args: [--fix]
      - id: ruff-format
""",
            "Dockerfile": """FROM python:3.11-slim AS builder
WORKDIR /app
COPY pyproject.toml .
RUN pip install --no-cache-dir .

FROM python:3.11-slim
WORKDIR /app
COPY --from=builder /usr/local /usr/local
COPY src/ src/
EXPOSE 8000
CMD ["uvicorn", "src.main:app", "--host", "0.0.0.0", "--port", "8000"]
""",
        },
    },

    # ── Python / FastAPI + PostgreSQL ─────────────────────────────────
    "fastapi-postgres": {
        "name": "FastAPI + PostgreSQL",
        "match_skills": ["rest-api-development", "sql-database"],
        "match_keywords": ["fastapi", "postgres", "database", "sqlalchemy"],
        "files": {
            "pyproject.toml": """[project]
name = "app"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = [
    "fastapi>=0.115",
    "uvicorn[standard]>=0.30",
    "pydantic>=2.0",
    "pydantic-settings>=2.0",
    "sqlalchemy>=2.0",
    "asyncpg>=0.29",
    "alembic>=1.13",
]

[project.optional-dependencies]
dev = ["pytest>=8.0", "httpx>=0.27", "ruff>=0.4", "pytest-asyncio>=0.23"]

[tool.pytest.ini_options]
testpaths = ["tests"]
asyncio_mode = "auto"
""",
            "src/__init__.py": '"""Application package."""\n',
            "src/main.py": """\"\"\"FastAPI application.\"\"\"
from fastapi import FastAPI
from src.config import settings

app = FastAPI(title=settings.app_name, version="0.1.0")


@app.get("/health")
async def health() -> dict:
    return {"status": "ok", "database": bool(settings.database_url)}
""",
            "src/config.py": """\"\"\"Configuration from environment.\"\"\"
from pydantic_settings import BaseSettings


class Settings(BaseSettings):
    app_name: str = "app"
    debug: bool = False
    database_url: str = "postgresql+asyncpg://localhost:5432/app"

    class Config:
        env_file = ".env"


settings = Settings()
""",
            "src/database.py": """\"\"\"Database connection and session management.\"\"\"
from sqlalchemy.ext.asyncio import AsyncSession, create_async_engine, async_sessionmaker
from src.config import settings

engine = create_async_engine(settings.database_url, echo=settings.debug)
async_session = async_sessionmaker(engine, class_=AsyncSession, expire_on_commit=False)


async def get_db():
    async with async_session() as session:
        yield session
""",
            "src/models.py": """\"\"\"SQLAlchemy models.\"\"\"
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column
from sqlalchemy import String


class Base(DeclarativeBase):
    pass


# Add your models here
# class Item(Base):
#     __tablename__ = "items"
#     id: Mapped[int] = mapped_column(primary_key=True)
#     name: Mapped[str] = mapped_column(String(255))
""",
            "tests/__init__.py": "",
            "tests/conftest.py": """import pytest
from fastapi.testclient import TestClient
from src.main import app

@pytest.fixture
def client():
    return TestClient(app)
""",
            "tests/test_health.py": """def test_health(client):
    r = client.get("/health")
    assert r.status_code == 200
    assert r.json()["status"] == "ok"
""",
            "docker-compose.yml": """services:
  app:
    build: .
    ports: ["8000:8000"]
    environment:
      DATABASE_URL: postgresql+asyncpg://app:app@postgres:5432/app
    depends_on:
      postgres: { condition: service_healthy }
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: app
      POSTGRES_PASSWORD: app
      POSTGRES_DB: app
    healthcheck:
      test: ["CMD", "pg_isready", "-U", "app"]
      interval: 5s
""",
            ".github/workflows/ci.yml": """name: CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:16-alpine
        env: { POSTGRES_USER: test, POSTGRES_PASSWORD: test, POSTGRES_DB: test }
        ports: ["5432:5432"]
        options: --health-cmd="pg_isready -U test" --health-interval=5s
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with: { python-version: "3.11" }
      - run: pip install -e ".[dev]"
      - run: ruff check src/ tests/
      - run: pytest -v
        env: { DATABASE_URL: "postgresql+asyncpg://test:test@localhost:5432/test" }
""",
            ".pre-commit-config.yaml": """repos:
  - repo: https://github.com/astral-sh/ruff-pre-commit
    rev: v0.4.0
    hooks:
      - id: ruff
        args: [--fix]
      - id: ruff-format
""",
        },
    },

    # ── Python / Data Science ─────────────────────────────────────────
    "python-data": {
        "name": "Python Data Science Project",
        "match_skills": ["data-analysis-pandas", "python-development"],
        "match_keywords": ["pandas", "data", "analysis", "csv", "anomaly", "ml"],
        "files": {
            "pyproject.toml": """[project]
name = "analysis"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = ["pandas>=2.0", "numpy>=1.26"]

[project.optional-dependencies]
dev = ["pytest>=8.0", "ruff>=0.4"]

[tool.pytest.ini_options]
testpaths = ["tests"]
""",
            "src/__init__.py": '"""Data analysis package."""\n',
            "tests/__init__.py": "",
            "tests/conftest.py": """import pytest
import pandas as pd

@pytest.fixture
def sample_df():
    return pd.DataFrame({
        "id": [1, 2, 3],
        "value": [10.0, 20.0, 30.0],
    })
""",
            "data/.gitkeep": "",
            ".github/workflows/ci.yml": """name: CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with: { python-version: "3.11" }
      - run: pip install -e ".[dev]"
      - run: pytest -v
""",
            ".pre-commit-config.yaml": """repos:
  - repo: https://github.com/astral-sh/ruff-pre-commit
    rev: v0.4.0
    hooks:
      - id: ruff
        args: [--fix]
""",
        },
    },

    # ── Python / CLI Tool ─────────────────────────────────────────────
    "python-cli": {
        "name": "Python CLI Tool",
        "match_skills": ["python-development"],
        "match_keywords": ["cli", "command", "argparse", "tool"],
        "files": {
            "pyproject.toml": """[project]
name = "tool"
version = "0.1.0"
requires-python = ">=3.11"

[project.scripts]
tool = "src.cli:main"

[project.optional-dependencies]
dev = ["pytest>=8.0", "ruff>=0.4"]

[tool.pytest.ini_options]
testpaths = ["tests"]
""",
            "src/__init__.py": '"""CLI tool package."""\n',
            "src/cli.py": """\"\"\"CLI entry point.\"\"\"
import argparse


def main():
    parser = argparse.ArgumentParser(description="Tool")
    parser.add_argument("command", help="Command to run")
    args = parser.parse_args()
    print(f"Running: {args.command}")


if __name__ == "__main__":
    main()
""",
            "tests/__init__.py": "",
            ".github/workflows/ci.yml": """name: CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with: { python-version: "3.11" }
      - run: pip install -e ".[dev]"
      - run: pytest -v
""",
        },
    },

    # ── TypeScript / Next.js ──────────────────────────────────────────
    "nextjs": {
        "name": "Next.js Application",
        "match_skills": ["typescript-development"],
        "match_keywords": ["nextjs", "next.js", "react", "frontend", "web app"],
        "files": {
            "package.json": """{
  "name": "app",
  "version": "0.1.0",
  "private": true,
  "scripts": {
    "dev": "next dev",
    "build": "next build",
    "start": "next start",
    "lint": "next lint",
    "test": "jest"
  },
  "dependencies": {
    "next": "^14",
    "react": "^18",
    "react-dom": "^18"
  },
  "devDependencies": {
    "@types/node": "^20",
    "@types/react": "^18",
    "typescript": "^5",
    "jest": "^29",
    "@testing-library/react": "^15",
    "eslint": "^8",
    "eslint-config-next": "^14"
  }
}
""",
            "tsconfig.json": """{
  "compilerOptions": {
    "target": "es5",
    "lib": ["dom", "dom.iterable", "esnext"],
    "strict": true,
    "jsx": "preserve",
    "module": "esnext",
    "moduleResolution": "bundler",
    "paths": { "@/*": ["./src/*"] }
  },
  "include": ["next-env.d.ts", "**/*.ts", "**/*.tsx"],
  "exclude": ["node_modules"]
}
""",
            "src/app/layout.tsx": """export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
""",
            "src/app/page.tsx": """export default function Home() {
  return <main><h1>App</h1></main>;
}
""",
            ".github/workflows/ci.yml": """name: CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: "20" }
      - run: npm ci
      - run: npm run lint
      - run: npm run build
""",
        },
    },

    # ── Rust / CLI ────────────────────────────────────────────────────
    "rust-cli": {
        "name": "Rust CLI Application",
        "match_skills": ["rust-development"],
        "match_keywords": ["rust", "cargo", "cli"],
        "files": {
            "Cargo.toml": """[package]
name = "app"
version = "0.1.0"
edition = "2024"

[dependencies]
clap = { version = "4", features = ["derive"] }
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
""",
            "src/main.rs": """use clap::Parser;

#[derive(Parser)]
#[command(name = "app", about = "Application")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Run the application
    Run,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run => println!("Running"),
    }
    Ok(())
}
""",
            ".github/workflows/ci.yml": """name: CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test
      - run: cargo clippy -- -D warnings
""",
        },
    },
}


def match_template(skills: list[str], task_text: str) -> str | None:
    """Find the best matching template for a task's skills and description."""
    best_match = None
    best_score = 0

    text = task_text.lower()

    for template_id, template in TEMPLATES.items():
        score = 0

        # Skill matches (weighted 2x)
        for skill in template["match_skills"]:
            if skill in skills:
                score += 2

        # Keyword matches
        for keyword in template["match_keywords"]:
            if keyword in text:
                score += 1

        if score > best_score:
            best_score = score
            best_match = template_id

    # Minimum score threshold
    return best_match if best_score >= 2 else None


def apply_template(worktree: str, template_id: str) -> list[str]:
    """Apply a template to a worktree. Returns list of files created."""
    template = TEMPLATES.get(template_id)
    if not template:
        return []

    created = []
    for filepath, content in template["files"].items():
        full_path = os.path.join(worktree, filepath)
        os.makedirs(os.path.dirname(full_path), exist_ok=True)

        # Don't overwrite existing files
        if not os.path.exists(full_path):
            Path(full_path).write_text(content)
            created.append(filepath)

    return created


def scaffold_project(worktree: str, skills: list[str], task_text: str) -> dict:
    """Match and apply the best template. Returns template info and files created."""
    template_id = match_template(skills, task_text)
    if not template_id:
        return {"template": None, "files": []}

    files = apply_template(worktree, template_id)
    template = TEMPLATES[template_id]

    return {
        "template": template_id,
        "template_name": template["name"],
        "files": files,
    }


# ── Self-test ───────────────────────────────────────────────────────────────

if __name__ == "__main__":
    import tempfile

    print("=== Template Matching ===\n")

    test_cases = [
        (["rest-api-development", "python-development"], "Build a FastAPI endpoint"),
        (["rest-api-development", "sql-database"], "Build FastAPI with PostgreSQL"),
        (["data-analysis-pandas"], "Analyze hotel rates from CSV, detect anomalies"),
        (["typescript-development"], "Build a Next.js web application"),
        (["rust-development"], "Build a CLI tool in Rust"),
        (["python-development"], "Build a CLI calculator"),
        (["python-development"], "Write some Python code"),  # too vague → no match
    ]

    for skills, text in test_cases:
        match = match_template(skills, text)
        name = TEMPLATES[match]["name"] if match else "(no match)"
        print(f"  [{', '.join(skills)[:30]}] '{text[:40]}' → {name}")

    print("\n=== Scaffold FastAPI+Postgres ===\n")

    with tempfile.TemporaryDirectory() as d:
        result = scaffold_project(d, ["rest-api-development", "sql-database"],
                                  "Build a FastAPI app with PostgreSQL")
        print(f"  Template: {result['template_name']}")
        print(f"  Files ({len(result['files'])}):")
        for f in sorted(result["files"]):
            print(f"    {f}")
