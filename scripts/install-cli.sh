#!/usr/bin/env bash
# Instala o unloop-cli para uso no terminal e por agentes (Claude Code, Orca etc.):
#   1. wrapper em ~/.local/bin/unloop-cli (aliases não funcionam em shells de agentes)
#   2. skill em ~/.claude/skills/unloop
#   3. permissões no ~/.claude/settings.json (mescladas, com backup)
#   4. opcional: backup diário automático (timer do systemd do usuário)
set -euo pipefail

APP_ID="io.github.joaovmourasouza.UnLoop"
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${HOME}/.local/bin"
WRAPPER="${BIN_DIR}/unloop-cli"
SKILL_DIR="${HOME}/.claude/skills/unloop"
SETTINGS="${HOME}/.claude/settings.json"
MARK="gerado por scripts/install-cli.sh (UnLoop)"

MODE="full"     # full | read-only | none
SOURCE="auto"   # auto | flatpak | caminho de um binário
BACKUP_TIMER=0

usage() {
    cat <<EOF
Uso: scripts/install-cli.sh [opções]

  --read-only        Agentes só podem LER sem pedir aprovação (escritas pedem confirmação).
                     Padrão: podem ler e escrever; apagar sempre pede confirmação.
  --no-permissions   Não mexe no ~/.claude/settings.json.
  --flatpak          Usa a CLI de dentro do Flatpak instalado.
  --bin CAMINHO      Usa um binário compilado (ex.: target/release/unloop-cli).
  --backup-timer     Ativa backup diário em ~/.local/share/unloop-backups (guarda 30 dias).
  -h, --help         Mostra esta ajuda.

Sem --flatpak/--bin: usa o Flatpak se estiver instalado; senão target/release/unloop-cli.
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --read-only) MODE="read-only" ;;
        --no-permissions) MODE="none" ;;
        --flatpak) SOURCE="flatpak" ;;
        --bin) SOURCE="${2:?informe o caminho do binário}"; shift ;;
        --backup-timer) BACKUP_TIMER=1 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "opção desconhecida: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }

# ------------------------------------------------------------ 1. Wrapper
if [ "$SOURCE" = "auto" ]; then
    if command -v flatpak >/dev/null 2>&1 && flatpak info "$APP_ID" >/dev/null 2>&1; then
        SOURCE="flatpak"
    elif [ -x "$REPO_DIR/target/release/unloop-cli" ]; then
        SOURCE="$REPO_DIR/target/release/unloop-cli"
    else
        echo "erro: não achei o Flatpak $APP_ID nem target/release/unloop-cli." >&2
        echo "      Instale o Flatpak ou rode: cargo build --release -p unloop-cli" >&2
        exit 1
    fi
fi

mkdir -p "$BIN_DIR"
if [ -e "$WRAPPER" ] && ! grep -q "$MARK" "$WRAPPER" 2>/dev/null; then
    cp "$WRAPPER" "$WRAPPER.bak.$(date +%Y%m%d%H%M%S)"
    say "Havia outro $WRAPPER; cópia de segurança criada."
fi

if [ "$SOURCE" = "flatpak" ]; then
    cat > "$WRAPPER" <<EOF
#!/bin/sh
# $MARK
exec flatpak run --command=unloop-cli $APP_ID "\$@"
EOF
    say "Wrapper criado: $WRAPPER → Flatpak $APP_ID"
else
    BIN="$(readlink -f "$SOURCE")"
    if [ ! -x "$BIN" ]; then
        echo "erro: $SOURCE não é um executável." >&2
        exit 1
    fi
    if [ "$BIN" = "$(readlink -f "$WRAPPER" 2>/dev/null || true)" ]; then
        echo "erro: --bin aponta para o próprio wrapper." >&2
        exit 1
    fi
    cat > "$WRAPPER" <<EOF
#!/bin/sh
# $MARK
exec "$BIN" "\$@"
EOF
    say "Wrapper criado: $WRAPPER → $BIN"
fi
chmod +x "$WRAPPER"

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "aviso: $BIN_DIR não está no seu PATH. Adicione ao ~/.profile:"
       echo "        export PATH=\"\$HOME/.local/bin:\$PATH\"" ;;
esac

# ------------------------------------------------------------- 2. Skill
mkdir -p "$SKILL_DIR"
cp "$REPO_DIR/skills/unloop/SKILL.md" "$SKILL_DIR/SKILL.md"
say "Skill instalado em $SKILL_DIR"

# -------------------------------------------------------- 3. Permissões
if [ "$MODE" != "none" ]; then
    if command -v python3 >/dev/null 2>&1; then
        python3 - "$SETTINGS" "$MODE" <<'PY'
import json, os, shutil, sys, time

path, mode = sys.argv[1], sys.argv[2]
read_only = [
    "context", "habit list", "habit show", "diary list", "note list", "note show",
    "rule list", "task list", "task show", "list all", "focus list", "focus stats",
    "where", "export", "--help", "--version",
]
deletes = ["diary delete", "note delete", "rule delete", "task delete", "list delete"]

def variants(cmd):
    return [f"Bash(unloop-cli {cmd}:*)", f"Bash(unloop-cli --json {cmd}:*)"]

if mode == "read-only":
    allow = [v for c in read_only for v in variants(c)]
    ask = []
else:
    allow = ["Bash(unloop-cli:*)"]
    ask = [v for c in deletes for v in variants(c)]

data = {}
if os.path.exists(path):
    with open(path, encoding="utf-8") as f:
        try:
            data = json.load(f)
        except json.JSONDecodeError as e:
            sys.exit(f"erro: {path} não é um JSON válido ({e}); corrija e rode de novo.")
    shutil.copy2(path, f"{path}.bak.{time.strftime('%Y%m%d%H%M%S')}")

perms = data.setdefault("permissions", {})
for key in ("allow", "ask"):
    perms[key] = [r for r in perms.get(key, []) if "unloop-cli" not in r]
perms["allow"].extend(allow)
perms["ask"].extend(ask)
if not perms["ask"]:
    del perms["ask"]

os.makedirs(os.path.dirname(path), exist_ok=True)
with open(path, "w", encoding="utf-8") as f:
    json.dump(data, f, indent=2, ensure_ascii=False)
    f.write("\n")
print(f"==> Permissões do Claude Code atualizadas ({mode}) em {path}")
PY
    else
        echo "aviso: python3 não encontrado; adicione manualmente ao $SETTINGS:"
        echo '        { "permissions": { "allow": ["Bash(unloop-cli:*)"] } }'
    fi
fi

# ---------------------------------------------------- 4. Backup diário
if [ "$BACKUP_TIMER" = 1 ]; then
    BACKUP_SCRIPT="$BIN_DIR/unloop-backup"
    cat > "$BACKUP_SCRIPT" <<'EOF'
#!/bin/sh
# gerado por scripts/install-cli.sh (UnLoop): backup diário, guarda 30 dias
set -e
dir="${XDG_DATA_HOME:-$HOME/.local/share}/unloop-backups"
mkdir -p "$dir"
file="$dir/unloop-$(date +%Y%m%d).json"
"$HOME/.local/bin/unloop-cli" export > "$file.tmp"
mv "$file.tmp" "$file"
find "$dir" -name 'unloop-*.json' -mtime +30 -delete
EOF
    chmod +x "$BACKUP_SCRIPT"

    UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
    mkdir -p "$UNIT_DIR"
    cat > "$UNIT_DIR/unloop-backup.service" <<'EOF'
[Unit]
Description=Backup diário do UnLoop

[Service]
Type=oneshot
ExecStart=%h/.local/bin/unloop-backup
EOF
    cat > "$UNIT_DIR/unloop-backup.timer" <<'EOF'
[Unit]
Description=Backup diário do UnLoop

[Timer]
OnCalendar=daily
Persistent=true

[Install]
WantedBy=timers.target
EOF
    systemctl --user daemon-reload
    systemctl --user enable --now unloop-backup.timer
    say "Backup diário ativado (systemctl --user list-timers unloop-backup.timer)"
fi

# ------------------------------------------------------------- Teste
say "Testando:"
"$WRAPPER" where || {
    echo "erro: o wrapper não funcionou; confira o caminho acima." >&2
    exit 1
}
echo
say "Pronto. Abra o Claude Code e peça, por exemplo: \"mostra meus hábitos no UnLoop\"."
