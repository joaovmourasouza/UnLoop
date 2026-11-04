# UnLoop

Rastreador de hábitos para GNOME/Linux: conta o tempo limpo, registra recaídas, metas, troféus, economia de dinheiro e tempo. **Offline e privado**: tudo fica num SQLite local, sem conta, sem nuvem, sem anúncios e sem telemetria.

Feito em **Rust + GTK4 + libadwaita** e distribuído como **Flatpak**. As regras de negócio vêm da análise em [docs/REA-Quitzilla.md](docs/REA-Quitzilla.md).

## Estrutura

```
crates/unloop-core/   regras, SQLite e backup, sem GUI (testável em qualquer máquina)
  src/model.rs        enums com códigos do Quitzilla, ícones (emoji), cores, entidades
  src/calc.rs         sequência, períodos, troféus, metas, saldos (+ testes)
  src/format.rs       formatação pt-BR de durações, números e moeda (+ testes)
  src/presets.rs      45 metas padrão, sugestões, moedas (+ testes)
  src/db.rs           schema + migrações (PRAGMA user_version) e repositório (+ testes)
  src/backup.rs       export/import JSON (compatível com o Quitzilla) e CSV (+ testes)
crates/unloop/        app GTK4/libadwaita
  src/app.rs          janela, contexto compartilhado, relógio de 1 s, ações, CSS
  src/home.rs         tela inicial
  src/habit_form.rs   criar/editar hábito
  src/detail.rs       detalhe com 7 abas
  src/entry.rs        entrada do diário
  src/settings.rs     configurações e backup
data/                 .desktop, metainfo e ícone
build-aux/flatpak/    manifest do Flatpak
```

## Desenvolvimento (Fedora/Ubuntu/Arch)

1. Instale o Rust (`curl https://sh.rustup.rs -sSf | sh`) e as bibliotecas de desenvolvimento:
   - Fedora: `sudo dnf install gtk4-devel libadwaita-devel gcc`
   - Ubuntu 24.04+/Debian 13: `sudo apt install libgtk-4-dev libadwaita-1-dev build-essential`
   - Arch: `sudo pacman -S gtk4 libadwaita base-devel`

   O libadwaita precisa ser 1.5 ou mais recente.
2. Rode os testes do núcleo (não precisam de GTK):
   ```bash
   cargo test -p unloop-core
   ```
3. Rode o app:
   ```bash
   cargo run -p unloop
   ```
   Fora do Flatpak, o banco fica em `~/.local/share/unloop/unloop.db`.

## Flatpak

```bash
flatpak install flathub org.gnome.Sdk//47 org.gnome.Platform//47 org.freedesktop.Sdk.Extension.rust-stable//24.08
flatpak install flathub org.flatpak.Builder
flatpak run org.flatpak.Builder --user --install --force-clean build-dir \
    build-aux/flatpak/io.github.joaovmourasouza.UnLoop.Devel.json
flatpak run io.github.joaovmourasouza.UnLoop
```

- O manifest `Devel` libera rede **só durante o build**, para o `cargo` baixar as dependências. O app instalado não tem permissão de rede nem de acesso a arquivos: importar e exportar usam o seletor de arquivos do sistema (portal).
- No Flatpak, o banco fica em `~/.var/app/io.github.joaovmourasouza.UnLoop/data/unloop/unloop.db`.
- Para publicar no Flathub, o build precisa ser offline. Gere o `cargo-sources.json` com o [flatpak-cargo-generator](https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo) a partir do `Cargo.lock`, troque `build-args` por `CARGO_NET_OFFLINE=true` e adicione `cargo-sources.json` em `sources`.

## Regras: permitido × não permitido

Cada hábito tem duas listas, na aba **Regras**:

- ✅ **Permitido**: não reinicia a sequência. Ex. (parar com refrigerante): refrigerante sem açúcar, água com gás.
- ⛔ **Não permitido**: reinicia a sequência. Ex.: refrigerante comum, energético com açúcar.

Ao registrar algo no diário, escolha a regra relacionada e o app decide sozinho se o contador reinicia. Sem regra, vale a escolha manual ("Foi uma recaída"). A aba mostra quantas vezes cada regra foi registrada e oferece sugestões para os hábitos mais comuns.

## Migrando do Quitzilla

No celular, faça um backup em arquivo (local). Copie o `.json` para o computador e use **Configurações → Restaurar backup**. Hábitos, diário, metas, recompensas, investimentos, motivos, afirmações, moeda e lema são importados. Os troféus são recalculados.

## Status (v0.1)

> O código ainda **não foi compilado**: foi escrito numa máquina Windows sem Rust e sem GTK. Rode `cargo test -p unloop-core` e `cargo build -p unloop` primeiro e corrija o que o compilador apontar.

Pronto: hábitos (criar, editar, apagar, reordenar, 12 formatos de contador, contagem progressiva ou regressiva), **regras por hábito** (o que é permitido e não reinicia a sequência, e o que não é permitido e reinicia; com sugestões prontas para hábitos comuns e vínculo com o diário), diário com filtros, metas padrão e pessoais (com fixação), estatísticas com gráfico de períodos, 12 troféus, recompensas, investimentos de tempo, projeção de economia, motivos e afirmações, tema, moeda, lema, backup JSON (inclusive do Quitzilla) e exportação do diário em CSV.

Próximos passos:
- [ ] Onboarding guiado (custo da vida toda, primeira meta, primeira recompensa)
- [ ] Notificações (meta alcançada, troféu, frase do dia) via portal, usando a tabela `trophy` para marcar o que já foi notificado
- [ ] Iniciar em segundo plano com a sessão (portal Background) para as notificações
- [ ] Celebração ao alcançar uma meta
- [ ] Senha opcional e banco cifrado (SQLCipher)
- [ ] Manifest offline para o Flathub
