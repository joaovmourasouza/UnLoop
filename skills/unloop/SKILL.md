---
name: unloop
description: Lê e modifica os dados do UnLoop (hábitos, diário, check-ins, notas de gatilhos, tarefas, metas em cascata ano→semana, foco) pela CLI `unloop-cli`. Use quando o usuário pedir para registrar algo no UnLoop, criar ou descrever tarefas, anotar gatilhos ou o que ajuda num hábito, ou analisar o progresso dele.
---

# UnLoop pela linha de comando

O UnLoop é o rastreador de hábitos e tarefas do usuário (app GNOME, dados num SQLite local). A CLI `unloop-cli` lê e grava no mesmo banco; o app aberto se atualiza sozinho.

## Como chamar

- Use `unloop-cli <comando>`. É um wrapper em `~/.local/bin` criado por `scripts/install-cli.sh` (aponta para o Flatpak ou para um binário compilado).
- Se `unloop-cli` não existir, use `flatpak run --command=unloop-cli io.github.joaovmourasouza.UnLoop <comando>` e sugira ao usuário rodar `scripts/install-cli.sh` no repositório do UnLoop. Aliases não funcionam aqui: o shell dos agentes não os carrega.
- **Sempre use `--json`** para ler dados. Para conferir qual banco está em uso: `unloop-cli where`.
- **Textos longos** (descrições, análises): passe pela entrada padrão com `-`, porque dentro do Flatpak a CLI não enxerga `/tmp` nem a home:
  ```bash
  unloop-cli note add "refrigerante" --kind insight --source claude --title "Padrão das sextas" --body-file - <<'EOF'
  ...markdown...
  EOF
  ```
- Outros agentes podem estar usando o UnLoop ao mesmo tempo: **sempre leia antes de criar** (evita tarefas e notas duplicadas).

## Regras de segurança

1. Antes de mudanças em massa (muitas tarefas, reescrever notas), faça backup redirecionando a saída (funciona com Flatpak): `unloop-cli export > ~/unloop-backup-$(date +%Y%m%d-%H%M).json`.
2. Apagar exige `--yes`. **Só apague com confirmação explícita do usuário**, citando o que será apagado.
3. Notas e análises escritas por você levam `--source claude` (o app mostra 🤖).
4. Não invente dados: leia antes (`context --json`, `habit show`, `task show`) e cite números reais.
5. Recaídas mudam a sequência do usuário: só registre `diary add --reset` quando ele disser que aconteceu.

## Comandos principais

```bash
unloop-cli --json context --days 30          # retrato completo: hábitos + estatísticas + notas + diário recente + tarefas + foco
unloop-cli --json habit list
unloop-cli --json habit show "refrigerante"  # id ou parte do nome

# Criar hábitos (confira antes com `habit list`: nome repetido dá erro, salvo com --force)
unloop-cli habit add "Cigarro" --since "12/11 22:00" --money 12 --minutes 30     # parar: contador desde a última vez
unloop-cli habit add "Acordar cedo" --kind build --days seg-sex                  # criar: check diário
unloop-cli habit add "Leitura" --kind build --target 30 --unit páginas --icon leitura --color azul
# --days: todos | seg-sex | fds | "seg,qua,sex"; --icon: emoji ou nome ("cigarro"); padrão: deduzido do nome

# Hábitos de parar (contador)
unloop-cli diary add "refrigerante" --note "vontade forte depois do almoço"          # anotação (não reinicia)
unloop-cli diary add "refrigerante" --reset --at "hoje 15:30" --money 7 --note "..."  # recaída
unloop-cli rule add "refrigerante" --kind allowed "Refrigerante zero"               # permitido (não reinicia)
unloop-cli rule add "refrigerante" --kind forbidden "Refrigerante comum"            # não permitido (reinicia)
unloop-cli diary add "refrigerante" --rule 3 --note "tomei um zero"                 # a regra decide se reinicia

# Hábitos de check diário
unloop-cli checkin "acordar cedo"                  # marca hoje
unloop-cli checkin "leitura" --add 15              # soma 15 (páginas) ao dia
unloop-cli checkin "estudar" --set 50 --date ontem # define o total de um dia
unloop-cli checkin "acordar cedo" --undo
unloop-cli checkin "acordar cedo" --missed       # "não feito": quebra a sequência na hora, mesmo hoje

# Notas do hábito: trigger (me leva a fazer), helper (me ajuda), note, insight (análise)
unloop-cli note add "refrigerante" --kind trigger --title "Pós-almoço" --body "Sono e tédio por volta das 14h." --tags "tarde,trabalho"
unloop-cli note add "refrigerante" --kind helper --title "Água com gás gelada" --body "..."
unloop-cli note add "refrigerante" --kind insight --source claude --title "Padrão das sextas" --body-file - < analise.md
unloop-cli --json note list --habit "refrigerante" --kind trigger
unloop-cli note edit 12 --body "..."

# Tarefas
unloop-cli task add "Preparar apresentação" --list Trabalho --due "sex 14:00" --priority high \
  --tags "cliente" --desc-file - --subtask "Rascunho" --subtask "Revisão" <<'EOF'
## Objetivo
...
## Pronto quando
...
EOF
unloop-cli --json task list --view today      # today | week | overdue | inbox | all | done
unloop-cli --json task show 42
unloop-cli task edit 42 --due amanhã --desc-file - <<'EOF'
...nova descrição...
EOF
unloop-cli task done 42
unloop-cli task sub add 42 "Enviar por e-mail"
unloop-cli list all | list add "Casa" | list rename Casa "Lar" | list delete Lar --yes

# Metas em cascata: ano → semestre → trimestre → mês → semana (cada uma com meta-mãe um nível acima)
unloop-cli --json goal now                    # metas dos períodos de hoje, do ano à semana
unloop-cli --json goal tree --year 2026       # árvore do ano
unloop-cli goal add "Curso: chegar na aula ~#085" --level ano --area Estudo
unloop-cli goal add "Curso: #052 → ~#085" --period T4 --parent 1       # períodos: 2026, S2, T4, nov, 11/2026, W47, atual
unloop-cli goal add "Aulas #064 a #067" --parent 4                     # sem nível: o de baixo da mãe, no período atual
unloop-cli goal add "Fechar o Módulo 2" --period nov --parent 2 --habit "acordar cedo" --desc-file - <<'EOF'
...
EOF
unloop-cli goal done 5 | goal drop 5 --note "remanejado pra W48" | goal reopen 5 | goal delete 5 --yes
unloop-cli task add "Aula #065" --goal 5 --due hoje   # tarefa ligada à meta (task edit 42 --goal nenhum desliga)

# Foco
unloop-cli --json focus stats --period week   # week | month | year (com comparação ao período anterior)
unloop-cli --json focus list --limit 20
```

Prazos aceitam: `hoje`, `amanhã`, `ontem`, `+3`, `seg`…`dom` (próxima ocorrência), `2026-03-12`, `12/03`, `12/03/2026`, com hora opcional (`14:30` ou `9h`). `--due nenhum` remove o prazo.

## Fluxos úteis

**Analisar gatilhos de um hábito**
1. `unloop-cli --json habit show "<hábito>"` e `unloop-cli --json diary list "<hábito>" --limit 100`.
2. Cruze: dias da semana e faixas do dia com mais recaídas (`relapsesByWeekday`, `relapsesByPeriodOfDay`), anotações do diário, regras mais usadas e notas de gatilho já existentes.
3. Escreva a conclusão como nota `insight` com `--source claude`. Seja concreto (números, exemplos do diário) e termine com 1–3 sugestões práticas.
4. Se encontrar gatilhos ou estratégias novas, proponha ao usuário antes de criar notas `trigger`/`helper` em nome dele.

**Planejar em cascata (como no second brain)**
1. `goal tree --json` e `goal now --json` antes de criar: não duplique metas.
2. Desça do macro para o micro: a meta do ano gera as do semestre, que geram as do trimestre, do mês e da semana. Cada meta filha usa `--parent` (a CLI recusa mãe de nível igual ou abaixo, ou período que não contém o da filha).
3. A semana ISO pertence ao mês da sua quinta-feira (W44 = 26/10–01/11 é de outubro).
4. O que não rodou ou foi remanejado é **riscado** (`goal drop --note "motivo → destino"`), não apagado: sai do progresso da mãe, mas fica no histórico.
5. Tarefas do dia ligam na meta da semana com `--goal`; o progresso sobe pela cascata (média das filhas).

**Organizar tarefas a partir de uma conversa**
- Quebre pedidos grandes em tarefas com título curto e acionável, descrição com contexto/objetivo/critério de pronto, subtarefas para os passos e prazo/prioridade quando o usuário indicar.
- Confira duplicatas com `task list --view all --json` antes de criar.

**Revisão semanal**
- `context --json` + `focus stats --period week --json` + `task list --view done --json`: resuma o que foi feito, sequências, foco e o que está atrasado; ofereça replanejar prazos.
