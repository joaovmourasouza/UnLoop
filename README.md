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
flatpak install flathub org.gnome.Sdk//50 org.gnome.Platform//50 org.freedesktop.Sdk.Extension.rust-stable//25.08
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

## Dois tipos de hábito

- **Parar um hábito (contador)**: conta o tempo desde a última recaída. Tem regras, diário, metas por duração, economia de dinheiro e tempo, recompensas e troféus.
- **Criar um hábito (check diário)**: você marca todo dia que fez. Ex.: acordar cedo.
  - **Dias programados**: só esses dias contam (ex.: acordar cedo de segunda a sexta). Dias fora do programa não quebram a sequência; marcar neles conta como bônus.
  - **Meta diária opcional**: ex.: leitura de 30 páginas ou estudo de 50 min. Você registra o progresso ao longo do dia (+10, +20…) e o dia só conta quando bate a meta.
  - Hoje nunca quebra a sequência enquanto o dia não acabou.
  - Check direto na tela inicial, calendário de 12 semanas para corrigir dias passados, sequência atual e melhor sequência, taxa de conclusão e troféus por dias seguidos.

## Foco (Pomodoro)

Botão **🍅 Foco** na tela inicial (Ctrl+F):

- **Pomodoro** (foco, pausa curta e pausa longa a cada N focos, com durações configuráveis e início automático opcional) ou **cronômetro**.
- O timer continua rodando enquanto você navega. Ao fim de cada fase há notificação do sistema.
- **Vínculo com hábito**: o foco concluído é creditado num hábito de check. Com meta em minutos (ex.: "Estudar 50 min"), os minutos somam no progresso do dia; sem meta, a sessão marca o dia como feito.
- **Estatísticas**: pomodoros e foco de hoje e do total, dias seguidos com foco, gráfico por semana/mês/ano com navegação entre períodos, distribuição por hábito, melhor horário (foco por hora do dia) e histórico de sessões por dia.
- Sessões com menos de 1 minuto não são registradas.

## Notas dos hábitos

Aba **Notas** em todo hábito, em quatro grupos: 🔥 **Gatilhos** (o que me leva a fazer), 🛡️ **O que me ajuda**, 📝 **Notas** e 🤖 **Análises**. Cada nota tem título, texto (Markdown), tags e origem: você ou o Claude.

## Tarefas

Botão **✅ Tarefas** (Ctrl+T):

- **Visões**: Hoje (com atrasadas), Próximos 7 dias (por dia), Caixa de entrada, Todas (por lista) e Concluídas.
- **Tarefa**: título, descrição longa, lista, prazo (com ou sem horário), prioridade, tags, subtarefas e hábito relacionado.
- **Adição rápida**: campo no topo de cada visão.
- Concluir com um clique e **🍅 Foco nesta tarefa**, que vincula o Pomodoro à tarefa.
- As tarefas de hoje também aparecem na tela inicial.

## CLI e Claude Code

`unloop-cli` lê e modifica tudo pela linha de comando, no mesmo banco do app. O app aberto se atualiza sozinho quando a CLI grava.

```bash
unloop-cli --help
unloop-cli --json context                    # retrato completo para análise
unloop-cli note add "refrigerante" --kind trigger --title "Pós-almoço" --body "Sono e tédio às 14h"
unloop-cli task add "Relatório" --list Trabalho --due "sex 14:00" --priority high --subtask "Rascunho"
unloop-cli checkin "leitura" --add 15
```

Todo comando aceita `--json`. Apagar exige `--yes`. `unloop-cli where` mostra o banco em uso; `--db` ou `$UNLOOP_DB` escolhem outro.

### Instalação para o terminal e para agentes (Claude Code, Orca…)

```bash
scripts/install-cli.sh                 # agentes leem e escrevem; apagar sempre pede confirmação
scripts/install-cli.sh --read-only     # agentes só leem sem pedir; qualquer escrita pede aprovação
scripts/install-cli.sh --backup-timer  # + backup diário em ~/.local/share/unloop-backups (30 dias)
```

O script:
1. Cria o wrapper `~/.local/bin/unloop-cli`, apontando para o Flatpak ou, sem ele, para `target/release/unloop-cli`; escolha com `--flatpak` ou `--bin CAMINHO`. Alias não serve, porque o shell dos agentes não carrega aliases.
2. Copia o skill para `~/.claude/skills/unloop`, e com isso ele vale em qualquer projeto.
3. Mescla as permissões no `~/.claude/settings.json` sem apagar o que já existe, guardando um backup do arquivo antes.

Pode rodar de novo para trocar de modo ou atualizar o skill.

Dentro do Flatpak a CLI não enxerga `/tmp` nem a sua home. Por isso, para textos longos use `--body-file -` ou `--desc-file -` (entrada padrão), e para backup use `unloop-cli export > arquivo.json`.

Vários agentes podem usar ao mesmo tempo: o SQLite em modo WAL aguenta, e o app se atualiza sozinho. O skill manda ler antes de criar, para evitar duplicatas.

Depois é só pedir em linguagem natural, por exemplo:
- "anota no UnLoop que café depois das 16h me faz querer refrigerante";
- "analisa meus gatilhos de refrigerante dos últimos 30 dias";
- "cria as tarefas do projeto X com descrição e prazos";
- "faz minha revisão da semana".

O skill orienta o Claude a fazer backup antes de mudanças grandes, a só apagar com sua confirmação e a marcar com 🤖 as notas que ele escreve.

## Migrando do Quitzilla

No celular, faça um backup em arquivo (local). Copie o `.json` para o computador e use **Configurações → Restaurar backup**. Hábitos, diário, metas, recompensas, investimentos, motivos, afirmações, moeda e lema são importados. Os troféus são recalculados.

## Status (v0.1)

> O código ainda **não foi compilado**: foi escrito numa máquina Windows sem Rust e sem GTK. Rode `cargo test -p unloop-core` e `cargo build -p unloop` primeiro e corrija o que o compilador apontar.

Pronto: **tarefas** (listas, prazos, prioridade, tags, subtarefas), **notas de gatilhos/ajuda/análises**, **CLI `unloop-cli` + skill para o Claude Code**, **hábitos de check diário** (dias programados, meta diária com progresso), **Pomodoro/cronômetro com estatísticas**, hábitos de parar (criar, editar, apagar, reordenar, 12 formatos de contador, contagem progressiva ou regressiva), **regras por hábito** (o que é permitido e não reinicia a sequência, e o que não é permitido e reinicia; com sugestões prontas para hábitos comuns e vínculo com o diário), diário com filtros, metas padrão e pessoais (com fixação), estatísticas com gráfico de períodos, 12 troféus, recompensas, investimentos de tempo, projeção de economia, motivos e afirmações, tema, moeda, lema, backup JSON (inclusive do Quitzilla) e exportação do diário em CSV.

Próximos passos:
- [ ] Onboarding guiado (custo da vida toda, primeira meta, primeira recompensa)
- [ ] Notificações (meta alcançada, troféu, frase do dia) via portal, usando a tabela `trophy` para marcar o que já foi notificado
- [ ] Iniciar em segundo plano com a sessão (portal Background) para as notificações
- [ ] Celebração ao alcançar uma meta
- [ ] Senha opcional e banco cifrado (SQLCipher)
- [ ] Manifest offline para o Flathub
- [ ] Lembrete diário para hábitos de check ainda não feitos
- [ ] Tarefas recorrentes (todo dia / toda segunda…) e lembretes de prazo
- [ ] Servidor MCP sobre a CLI (para o Claude Desktop)
- [ ] Guardar o estado do timer ao fechar o app (hoje uma sessão em andamento se perde ao sair)
