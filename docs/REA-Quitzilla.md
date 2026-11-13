# Engenharia reversa: Quitzilla 3.0.2 → UnLoop (Linux)

Fonte: `Quitzilla_+Quit+Habit+Tracker_3.0.2_APKPure.apk` (pacote `com.despdev.quitzilla`, versão 3.0.2).
Método: leitura da tabela de strings dos 3 `classes*.dex`, do `resources.arsc` e do `AndroidManifest.xml`, mais descompilação das classes do app com androguard (DAD).

Legenda: **[código]** = confirmado no bytecode descompilado · **[strings]** = deduzido de strings/nomes de classe · **[inferido]** = hipótese razoável, não verificada.

> Reimplemente o comportamento, não o conteúdo. Não reutilize nome, ícones, imagens, textos de frases/afirmações/motivos nem a marca do app original.

---

## 1. Arquitetura e dependências originais

### 1.1 Tecnologia
- **Android nativo em Kotlin**: não há `libflutter.so`, bundle JS nem runtime RN. Coroutines e Flow em toda a camada de dados. **[código]**
- UI **híbrida**: Jetpack Compose (Material 3) nas telas novas (onboarding, backup, widgets) e Views/AppCompat/RecyclerView/ConstraintLayout nas telas antigas. **[strings]**
- Ofuscação via R8: classes de UI e de serviço preservam o nome (`ActivityMain`, `WorkerTrophyCheck`…), mas modelos, DAOs e repositório foram movidos para pacotes curtos (`j4`, `k4`, `n4`, `f4`, `i4`).

| Camada | Lib original | Onde |
|---|---|---|
| Persistência | **Room** (schema v3), arquivo `quitzilla.db` | `database.AppDatabase`, DAOs em `i4.*` |
| Preferências | **Jetpack DataStore (Preferences)** | chaves `KEY_*` |
| Jobs em background | **WorkManager** + `AlarmManager` (alarmes exatos) | `workers.*`, `notifications.AlarmReceiver` |
| Widgets | **Glance** (contador e frase do dia) | `widget.*` |
| Gráficos | **MPAndroidChart** (`com.github.mikephil`) | `ActivityHabitStatisticCharts` |
| Calendário | `com.kizitonwose.calendar` | diário |
| Serialização | Gson | backup JSON |
| Segurança | AndroidX Biometric | bloqueio do app |

### 1.2 SDKs de terceiros a **remover/substituir**

| SDK | Função no original | Ação no Linux |
|---|---|---|
| Firebase Analytics, Crashlytics, Remote Config, Sessions | telemetria, crash, flags remotas | **remover** |
| Google Mobile Ads (AdMob) + UMP (consentimento) + Ad ID | anúncios | **remover** |
| Google Play Billing + **RevenueCat** (`api.revenuecat.com`) | assinatura premium, paywall | **remover**: tudo liberado |
| Google Drive API + Google Auth | backup na nuvem (`appDataFolder`) | **trocar** por export/import de arquivo JSON local |
| Android Auto-Backup | backup do sistema | **trocar** por backup local agendado (opcional) |
| Play In-App Review, Play Services | avaliação na loja | **remover** |
| AndroidX Biometric | trava do app | senha local opcional (+ SQLCipher, se quiser cifrar o banco) |
| Glance widgets | widgets na tela inicial | ícone de bandeja (tray) e/ou notificação persistente |
| WorkManager / AlarmManager | troféus, metas, notificações | timer interno do app + `libnotify` (ou timer de usuário do systemd) |
| Links "More apps", Instagram, promo | marketing cruzado | **remover** |

Permissões relevantes do original: `INTERNET`, `POST_NOTIFICATIONS`, `SCHEDULE_EXACT_ALARM`, `RECEIVE_BOOT_COMPLETED`, `USE_BIOMETRIC`, `AD_ID`, `ACCESS_ADSERVICES_*`. A versão Linux precisa só de notificações. Rede: nenhuma.

---

## 2. Telas e fluxo de UI

### 2.1 Inventário de telas **[strings]**

| Activity original | Função |
|---|---|
| `ActivityLauncher` | roteador: primeira execução → onboarding; senão → bloqueio biométrico (se ativo) → Main |
| `onboarding.ActivityOnboarding` | funil de onboarding (ver 2.2) |
| `onboarding.habitcreation.ActivityHabitCreation` | criar hábito personalizado (nome + ícone) |
| `ActivityMain` | lista de hábitos ("I commit to quit:"), lema pessoal, total economizado |
| `ActivityOverview` | detalhe do hábito, com abas |
| `ActivityHabitSettings` | editar hábito (nome, ícone, cor, tipo/estilo do contador, gastos) |
| `ActivityHabitMetricEditor` | editar dinheiro/tempo/frequência por dia |
| `ActivityCounterStylePicker` | escolher o formato do contador |
| `ActivityHabitStatisticCharts` | gráficos (períodos de abstinência, gastos) |
| `ActivityDiary` / `ActivityDiaryEntry` | diário: lista filtrável + criar/editar entrada (com reset) |
| `ActivityGoals` / `ActivityGoalCreation` | metas (Próximas/Concluídas) + meta pessoal por data |
| `ActivityAffirmations` | afirmações da meta atual (mínimo 2) |
| `ActivityReasonEdit` | motivos para parar |
| `ActivityCurrentBalance` | extrato: acumulado − resets − recompensas compradas |
| `ActivityPotentialSavings` | projeção de economia até uma data futura |
| `ActivityPreferences` | configurações |
| `backup.ActivityBackup` | backup e restauração (arquivo ou Google Drive) |
| `premium.*` | paywall/trial: **descartar** |
| `widget.WidgetConfigureActivity*` | configurar widgets: **descartar ou trocar pelo tray** |

### 2.2 Fluxo de onboarding **[strings]**
1. Boas-vindas ("Break free from the loop")
2. Escolher hábito (lista de presets + "Create my own habit")
3. Há quanto tempo tem o hábito (meses/anos) → usado no "custo da vida toda"
4. Quanto dinheiro gasta (por dia/semana/mês, convertido para por dia)
5. Quanto tempo gasta (min/dia) e frequência (vezes/dia)
6. **Custo da vida toda**: dinheiro perdido, tempo perdido, ocorrências estimadas (= duração × taxas diárias)
7. Reframe ("o passado ficou para trás") e projeção de economia (1 mês, 1 ano, 5 anos, 10 anos)
8. "Quando foi a última vez?" → define `quitDate`
9. Motivos para parar (4 sugestões por hábito + personalizado)
10. Primeira meta (recomendada ou personalizada)
11. Primeira recompensa (presets ou personalizada) e primeiro investimento de tempo (leitura, exercício, diário)
12. Resumo "Changes have already begun" com o timer já rodando
13. Pedido de permissão de notificações
14. (Paywall/trial: descartar)

### 2.3 Navegação principal
```
Main (lista de hábitos, reordenável)
 ├─ [+] → criar hábito (fluxo curto do onboarding)
 ├─ Settings → Preferences → Backup / Export CSV / Tema / Idioma / Notificações / Biometria / Moeda / Data de nascimento
 └─ toque no hábito → Overview
       ├─ aba Overview   : contador grande + meta atual (ArcProgress) + afirmação + "Reset Timer" + última vez
       ├─ aba Statistics : progresso, dinheiro, tempo → "View Charts"
       ├─ aba Trophies   : grade de 11 troféus com data de conquista
       ├─ aba Incentives : Rewards (comprar com o saldo) | Investments (investir tempo) | Savings → Potential Savings / Current Balance
       ├─ aba Motivation : motivos para parar, frase do dia (aparece ao sacudir o aparelho)
       └─ menu: Diary · Goals · Share Progress (card em imagem: Portrait/Square/Story) · Habit Settings · Delete
```

### 2.4 Componentes visuais
- **Contador**: atualizado em tempo real. Dois **tipos** (`counterType`):
  - `ABSTINENCE` (300, padrão): conta para frente a partir do início da sequência atual.
  - `REMAINING` (200): conta para trás até a meta atual ("Time to Goal").

  Doze **estilos** (`counterStyle`, abaixo). A formatação usa `MeasureFormat` do ICU.
- **ArcProgress**: arco de progresso da meta atual (fórmula em 3.5), colorido com o `hexColor` da meta.
- **Celebração de meta**: "Congratulations!" + lista das metas atingidas + próxima meta. Para não repetir a celebração, o app guarda `lastCelebratedGoals` (mapa hábito → meta).
- **Troféus**: grade de cartões com ícone, nome do período e "Achieved · data".
- **Estatísticas**: dia em que parou, nº de resets, períodos de abstinência (mín/máx/média/anterior), dinheiro e tempo gastos no hábito, recompensas compradas, tempo investido.
- **Gráficos**: barras dos períodos de abstinência e de gastos, com filtro All / Last 10 / Last 25 e linha de média.
- **Diário**: lista com filtro All / Resets / With notes. Cada entrada tem data, gasto, tempo, comentário e a flag de reset ("By resetting, you indicate a relapse").
- **Age Clock** [inferido]: card que usa a data de nascimento ("Life is short").

---

## 3. Dados e regras de negócio

### 3.1 Onde os dados ficam no original
- `quitzilla.db` (SQLite via **Room**, `version = 3`, identity hash `effe7e4b…`). **[código]**
- **DataStore Preferences** para configurações. **[código]**
- **Não há chaves estrangeiras**: os DAOs apagam os filhos manualmente (`DELETE FROM X WHERE addictionId = ?`). Datas são epoch **em milissegundos** (`INTEGER`).

### 3.2 Esquema original (literal, **[código]**)
```sql
CREATE TABLE Addictions (_id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL, tv_text TEXT NOT NULL, icon INTEGER NOT NULL,
  color INTEGER NOT NULL, quitDate INTEGER NOT NULL, dailySpending REAL NOT NULL, positionInTheList INTEGER NOT NULL,
  preferredSpendingType INTEGER NOT NULL DEFAULT 0, counterType INTEGER NOT NULL DEFAULT 300,
  counterStyle INTEGER NOT NULL DEFAULT 400, dailyTImeSpending REAL NOT NULL, dailyFrequency REAL NOT NULL DEFAULT 0);
CREATE TABLE UsageEvents (_id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL, timestamp INTEGER NOT NULL, addictionId INTEGER NOT NULL,
  moneySpent REAL NOT NULL, timeSpent INTEGER NOT NULL, isReset INTEGER NOT NULL DEFAULT 1, comment TEXT);
CREATE TABLE Goals (_id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL, addictionId INTEGER NOT NULL, name TEXT NOT NULL,
  durationMs INTEGER NOT NULL, hexColor TEXT NOT NULL, isPersonal INTEGER NOT NULL DEFAULT 0, isForceCurrent INTEGER NOT NULL DEFAULT 0);
CREATE TABLE Trophies (TrophyId INTEGER NOT NULL, addictionId INTEGER NOT NULL, timePeriod INTEGER NOT NULL,
  is_achieved INTEGER NOT NULL, achieved_at INTEGER, PRIMARY KEY(TrophyId, addictionId));
CREATE TABLE Rewards (_id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL, tv_text TEXT NOT NULL, tv_position REAL NOT NULL,
  addictionId INTEGER NOT NULL, isPurchased INTEGER NOT NULL, isDeleted INTEGER NOT NULL, listPosition INTEGER NOT NULL DEFAULT 0, purchaseDate INTEGER);
CREATE TABLE TimeInvestment (_id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL, investmentName TEXT NOT NULL, addictionId INTEGER NOT NULL,
  singleSessionDuration INTEGER NOT NULL, totalInvestments INTEGER NOT NULL, isDeleted INTEGER NOT NULL, listPosition INTEGER NOT NULL DEFAULT 0);
CREATE TABLE ReasonsToQuit (_id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL, addictionId INTEGER NOT NULL, tv_text TEXT NOT NULL, listPosition INTEGER NOT NULL DEFAULT 0);
CREATE TABLE Affirmations (_id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL, addictionId INTEGER NOT NULL, tv_text TEXT NOT NULL, listPosition INTEGER NOT NULL DEFAULT 0);
-- + índices em addictionId de todas as tabelas e index_UsageEvents_habit_reset_ts (addictionId, isReset, timestamp)
```

Pegadinhas de nomenclatura no original:
- `tv_text` é o **nome**.
- `Rewards.tv_position` é o **preço**: o getter `g()` desse campo é somado como dinheiro. **[código]**
- `TimeInvestment.totalInvestments` é o **total de ms investidos** (somado como tempo). **[código]**
- `dailySpending` é **dinheiro/dia**; `dailyTImeSpending` (com o "I" maiúsculo, erro de digitação no original) é **minutos/dia**; `dailyFrequency` é **vezes/dia**.
- `Goals.durationMs`: em metas padrão é uma **duração**; em metas pessoais (`isPersonal=1`) é um **timestamp absoluto** da data-alvo. **[código]**
- `UsageEvents` é o **diário**. `isReset=1` é recaída (reinicia o timer); `isReset=0` é uma anotação ou um deslize que não zera o contador.

### 3.3 Enums (códigos estáveis, **[código]**)

| Enum | Valores (nome = código) |
|---|---|
| `counterType` | ABSTINENCE=300 (padrão), REMAINING=200 |
| `counterStyle` | FULL=400 (padrão, compacto), FULL_SHORT=401 (rotulado), DAYS_HOURS_MINUTES=411, DAYS_HOURS=402, WEEKS_DAYS=403, MONTHS_DAYS=404, YEARS_MONTHS_DAYS=405, MINUTES=412, HOURS=406, DAYS=407, WEEKS=408, MONTHS=409 |
| `preferredSpendingType` | MONEY=0, TIME=1 |
| `color` (21) | RED=1001, GREEN=1002, BLUE=1003, PINK=1004, PURPLE=1005, INDIGO=1006, CYAN=1007, TEAL=1008, YELLOW=1009, ORANGE=1010, BROWN=1011, BLUE_GRAY=1012, SCARLET=1013, SALMON=1014, ROYAL_BLUE=1015, BLUE_VIOLET=1016, LAVENDER=1017, NAVY=1018, MAUVE=1019, PLUM=1020, GRAY=1021 |
| `icon` (59) | GENERIC=2001, SMOKING=2002, ALCOHOL=2003, DRUGS=2004, PILLS=2005, CANNABIS=2006, FASTFOOD=2007, SUGAR=2008, PORN=2009, GAMES=2010, TELEVISION=2011, SHOPPING=2012, FACEBOOK=2013, TWITTER=2014, INSTAGRAM=2015, GAMBLING=2016, REDDIT=2017, YOUTUBE=2018, OVEREATING=2019, SODA=20120 (sic), COFFEE=2021, PROCRASTINATION=2022, CURSING=2023, EATING_MEAT=2024, LIE=2025, QUARREL=2026, BITING_FINGERNAILS=2027, … 2028–2059 (ícones genéricos: BUS, CAR, CASH, PHONE, PIZZA, EMOTION_*, TIKTOK=2056, VAPING=2057, SNUS=2058, SELF_HARM=2059) |
| Moedas (51) | ISO 4217: AED, ARS, AUD, … BRL, EUR, USD, … ZAR |

**Troféus** (`TrophyId` → período):

| TrophyId | Nome | Limiar |
|---|---|---|
| 1 | DECISION_MADE | conquistado no próprio `quitDate` |
| 2 | DAY_1 | 1 dia |
| 3 | DAY_3 | 3 dias |
| 4 | DAY_7 | 7 dias |
| 5 | DAY_10 | 10 dias |
| 6 | DAY_14 | 14 dias |
| 7 | DAY_30 | 30 dias |
| 8 | DAY_90 | 90 dias |
| 9 | DAY_180 | **182,5 dias** (15 768 000 000 ms) |
| 10 | DAY_365 | 365 dias |
| 11 | DAY_1825 | 1825 dias |

O enum de períodos tem ainda `DAY_3650` (10 anos), sem troféu associado nesta versão.

**Metas padrão**: 45 presets (`n4.e`), cada um com `durationMs` e uma cor de gradiente.
- Dias: 1, 2, 3, 4, 5, 6, 10, 50, 100, 200, 250, 300, 400, 500, 700, 1000, 2000, 3000.
- Semanas: 1, 2, 3, 5, 6, 10.
- Meses (1 mês = 30,4375 d): 1, 2, 3, 4, 5, 7, 8, 9, 10, 11, 15.
- Anos (1 ano = 365,25 d): 1 a 10.
- A cor vai de `#FC5C65` (1 dia) passando por roxo e azul até verde (`#00BD9E` em 1 ano) e termina em `#4B6584` (10 anos).

### 3.4 Preferências (DataStore) **[código]**
`KEY_APP_THEME`, `KEY_BIOMETRIC_LOCK_ENABLED`, `KEY_CHART_PERIODS_LIMIT`, `KEY_CHART_SPENDING_LIMIT`, `KEY_FIRST_LAUNCH`, `KEY_FIRST_LAUNCH_FOR_QUOTES`, `KEY_GOAL_CELEBRATION_INITIALIZED`, `KEY_HABIT_ITEM_CLICK_DISCOVERED`, `KEY_INSTALL_STAMP`, `KEY_RESTORE_IN_PROGRESS`, `KEY_SHAKE_QUOTE_DISCOVERED`, `KEY_USER_BIRTHDATE`, além de `currencyCode`, `mainTitle` (lema pessoal), `notificationsMotivation`, `notificationsProgress` e `lastCelebratedGoals` (mapa). Estas últimas aparecem no `BackupSettings`.

### 3.5 Regras e fórmulas **[código]**

Notação: `now` = agora (ms). `resets` = timestamps de `UsageEvents` com `isReset=1`, em ordem crescente. `DAY` = 86 400 000 ms.

**Início da sequência atual (streak start)**: SQL literal do DAO:
```sql
SELECT COALESCE((SELECT MAX(timestamp) FROM UsageEvents WHERE addictionId=? AND isReset=1),
                (SELECT quitDate FROM Addictions WHERE _id=?), 0)
```
- `streakStart` = último reset ou, se não houver, `quitDate`.
- **Tempo limpo atual** = `now − streakStart`.
- Não existe "streak" armazenado: tudo é derivado, nada fica em cache no banco.

**Períodos de abstinência (estatísticas)**:
```
prev = quitDate
para cada r em resets (crescente): periods += max(0, r − prev); prev = r
max = max(periods) · min = min(periods) · média = avg(periods) · anterior = último(periods) · nº de resets = len(resets)
```
O período em andamento **não** entra na lista. Sem resets, a tela mostra "—".

**Troféus** (`f4.h.b(quitDate, resets, now)`):
```
pontos = [quitDate] + sorted(r em resets se quitDate ≤ r ≤ now) + [now]
para cada troféu T (exceto DECISION_MADE):
    achievedAt(T) = primeiro i em que pontos[i+1] − pontos[i] ≥ T.ms  →  pontos[i] + T.ms
                  = null se nenhum intervalo alcançar o limiar
DECISION_MADE.achievedAt = quitDate
```
- Um troféu fica conquistado se **qualquer** período (passado ou atual) atingiu o limiar. Uma recaída **não** retira o troféu.
- A data registrada é o momento exato em que o limiar foi cruzado pela primeira vez.
- `WorkerTrophyCheck` só grava troféus ainda não conquistados (`UPDATE … SET is_achieved=1, achieved_at=?`).
- `WorkerTrophyRecompute` recalcula todos (por exemplo, depois de editar ou apagar um reset).

**Metas** (`j4.e`):
```
alvo(meta)      = isPersonal ? durationMs (timestamp absoluto) : streakStart + durationMs
atingida(meta)  = now ≥ alvo
progresso       = clamp((now − streakStart) / max(alvo − streakStart, 1), 0, 1)
restante        = max(alvo − now, 0)
meta atual      = a meta com isForceCurrent=1, se ainda não foi atingida;
                  senão, a primeira não atingida ordenando por (alvo, _id)
```
- Só uma meta pode ter `isForceCurrent=1`: o app zera todas e marca uma.
- Metas padrão não podem ser apagadas: `DELETE … WHERE isPersonal = 1`.
- **[inferido]** As opções "From now" e "From the last reset" da meta pessoal definem a base do progresso.

**Dinheiro** (`ActivityOverview`, aba Incentives):
```
taxaDinheiro      = dailySpending / DAY                          (por ms)
economiaTotal     = taxaDinheiro × (now − quitDate)               "Savings from the beginning"
economiaDesdeReset= taxaDinheiro × (now − streakStart)            "Savings since the last reset"
gastoNoHabito     = Σ UsageEvents.moneySpent (todas as entradas)
líquido           = economiaTotal − gastoNoHabito
saldoAtual        = líquido − Σ Rewards.price onde isPurchased=1  (inclui recompensas compradas e depois apagadas)
```
A compra de uma recompensa exige `saldoAtual ≥ preço` ("Not enough money"). O texto "You'll reach this reward in X" sai de `(preço − saldo) / taxaDinheiro`.

**Tempo** (mesma lógica):
```
taxaTempo     = dailyTimeSpending(min) × 60 000 / DAY
tempoTotal    = taxaTempo × (now − quitDate);   tempoDesdeReset = taxaTempo × (now − streakStart)
saldoTempo    = tempoTotal − Σ UsageEvents.timeSpent − Σ TimeInvestment.totalInvestments (inclui os apagados)
investir      = totalInvestments += singleSessionDuration   (se saldoTempo ≥ sessão; senão, "Not enough time")
```

**Tela inicial**: soma de `taxaDinheiro × max(0, now − quitDate)` de todos os hábitos (valor bruto, sem descontar gastos).

**Economia potencial**: `taxa × (dataFutura − now)` somada ao valor economizado até hoje ("Savings to Date").

**Custo da vida toda** (onboarding): `duração do hábito × {dailySpending, dailyTimeSpending, dailyFrequency}`.

**Entrada do diário**: o usuário escolhe a data e a hora do dia é preenchida com a hora atual. Não pode estar no futuro. `isReset` vem **ligado** por padrão.

**Exclusão de hábito**: apaga o hábito e todos os filhos (diário, metas, troféus, motivos, afirmações, recompensas, investimentos).

**Limites do free tier** (descartar): até 2 hábitos e um número máximo de recompensas e investimentos.

### 3.6 Formato de backup (JSON, `BackupData`) **[código]**
```
{ version, appVersion, createdAt,
  habits[{id,name,icon(nome do enum),color(nome),quitDate,dailyMoneySpending,dailyTimeSpending,dailyFrequency,
          listPosition,preferredSpendingType,counterType,counterStyle}],
  diaryEntries[{id,habitId,timestamp,moneySpent,timeSpent,isReset,comment}],
  goals[{id,habitId,name,durationMs,hexColor,isPersonal,isForceCurrent}],
  trophies[{habitId,milestone,timePeriod,isAchieved,achievedAt}],
  rewards[…], timeInvestments[…], reasons[…], affirmations[…],
  settings{appTheme,biometricLockEnabled,currencyCode,mainTitle,userBirthdate,notifications*,lastCelebratedGoals,…} }
```
➡ Vale a pena o UnLoop **importar esse JSON**: o usuário migra o histórico do celular sem nuvem (o backup "File on device" do Quitzilla gera exatamente esse arquivo).

---

## 4. Guia de migração para Linux desktop

### 4.1 Escolha de stack (decidida: Rust + GTK4/libadwaita, Flatpak)
- **UI**: GTK4 + libadwaita (`gtk4-rs` 0.9, `libadwaita-rs` 0.7), com a aparência nativa do GNOME (tema, modo escuro e acessibilidade do sistema).
- **Banco**: `rusqlite` com SQLite embutido (`bundled`).
- **Empacotamento**: Flatpak sobre `org.gnome.Platform` 50 + extensão `rust-stable`. Sem permissão de rede nem de arquivos; importação e exportação passam pelo portal do seletor de arquivos.
- **Ícones dos hábitos**: emoji, que dispensam assets e funcionam em qualquer tema. Os códigos numéricos continuam os do Quitzilla.
- **Workspace em duas crates**: `unloop-core`, Rust puro com as fórmulas da seção 3.5, o banco, o backup e os testes, e `unloop`, a GUI. O núcleo é testável com `cargo test -p unloop-core` sem GTK.

Alternativas consideradas: Flutter (UI Material, sem a aparência do GNOME), Tauri (UI web) e Python + GTK (mais lento).

### 4.2 Camadas
```
domain/        entidades + enums (códigos estáveis) + funções puras: streakStart, periods, trophies, currentGoal, balances
data/          SQLite (schema.sql), repositórios, import/export JSON e CSV
state/         providers/stores: HabitList, HabitDetail(habitId) → derivam tudo de (habit, events, goals, rewards, investments, now)
ui/            telas da seção 2
platform/      notificações, tray, autostart (~/.config/autostart/*.desktop), caminho XDG do banco
```
- **Banco**: `$XDG_DATA_HOME/unloop/unloop.db` (padrão `~/.local/share/unloop/`). Ative `PRAGMA foreign_keys=ON` e `journal_mode=WAL`.
- **Relógio**: um `ticker` de 1 s alimenta o contador. As agregações (estatísticas, troféus) só são recalculadas quando os dados mudam ou a cada minuto. Guarde tudo em **UTC epoch ms** e converta para o fuso local só na exibição.
- **Troféus e metas**: não precisa de WorkManager. Na abertura do app, a cada minuto e depois de qualquer mudança no diário, rode `computeTrophies()` e compare com a tabela; os que viraram "conquistados" geram notificação. Como o cálculo é determinístico, você pode até **não armazenar troféus** e derivá-los sempre. A tabela serve só para saber o que já foi notificado.
- **Notificações com o app fechado**: rode o app minimizado na bandeja, com autostart. Alternativa: um timer de usuário do systemd chamando `unloop --check`.
- **Widgets** → ícone de bandeja com tooltip "12d 4h" e/ou notificação persistente. Extensão do GNOME/KDE só se quiser ir além.
- **Biometria** → senha opcional (Argon2) e, opcionalmente, banco cifrado com SQLCipher.
- **Backup** → export/import JSON (compatível com o formato da seção 3.6) e export CSV. Backup automático rotativo em `~/.local/share/unloop/backups/` é opcional.
- **Moeda e locale**: formatação pt-BR própria em `unloop-core/src/format.rs`; datas locais via `glib::DateTime`. Estilos de contador conforme a tabela da seção 3.3.

### 4.3 Esquema proposto
O DDL completo está em [`schema.sql`](schema.sql). Diferenças em relação ao original:
- Nomes de coluna claros (`name`, `price`, `invested_ms`, `daily_minutes`) e `FOREIGN KEY … ON DELETE CASCADE`, em vez de apagar filhos manualmente.
- Enums gravados como os **mesmos códigos inteiros** do original, o que facilita importar backups do Quitzilla.
- Meta pessoal com coluna própria (`target_at`) em vez de reaproveitar `durationMs`.
- `CHECK`s de integridade e índice `(habit_id, is_reset, occurred_at)` para o `streakStart`.

### 4.4 Checklist de paridade
- [ ] CRUD de hábito + reordenação + 12 estilos e 2 tipos de contador
- [ ] Diário com reset, gasto, tempo, nota e filtros
- [ ] Estatísticas e gráficos (Last 10/25/All)
- [ ] 11 troféus com data de conquista
- [ ] 45 metas padrão + metas pessoais + "definir como atual" + celebração
- [ ] Recompensas (comprar com o saldo) e investimentos de tempo
- [ ] Saldo atual, economia potencial, custo da vida toda
- [ ] Motivos, afirmações (mínimo 2), frase do dia (atalho no lugar do "sacudir")
- [ ] Onboarding (sem paywall)
- [ ] Notificações (meta, troféu, frase do dia), tray, autostart
- [ ] Import do JSON do Quitzilla, export JSON/CSV
- [ ] Tema claro/escuro/sistema, idioma, moeda, lema, data de nascimento
