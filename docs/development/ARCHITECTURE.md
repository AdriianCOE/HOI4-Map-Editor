# Arquitetura do HOI4 Map Editor

HOI4 Map Editor e o nome publico do projeto. O crate/binario
`hoi4_map_editor`, o diretorio de backup `.hoi4-state-editor`, `MapViewMode`
e `ViewMode` permanecem como nomes tecnicos de compatibilidade; nao representam
uma segunda marca publica.

## Base herdada

O projeto e um fork do HOI4 Province Editor de ScottyThePilot. A inicializacao
continua em `main.rs`; `events.rs` executa o loop Piston; `App` traduz entradas
em acoes; `Canvas` reune sessao visual, camera, ferramentas e texturas;
`Bundle`, `Map` e `History` mantem os dados geograficos e undo/redo; e
`map/bridge.rs` usa `util/files.rs` para ler pastas ou ZIPs.

## Fronteira de classificacao de entrada

`app::input` e uma camada pura entre os eventos Piston e as acoes existentes.
Ela recebe um evento bruto pequeno mais `InputContext` (somente captura de UI,
workspace, bloqueio de edicao e presenca de Canvas) e retorna enums de comando.
Nao importa `App` ou `Canvas`, nao abre projetos, nao inicia Save e nao conhece
pixels, provincias, states ou sessoes de edicao. `App` coleta o contexto, roteia
o comando e chama as APIs ja existentes; `Canvas` continua sendo o executor e
proprietario de ferramentas, selecao, camera e historico.

`app::selection_navigation` acrescenta uma fronteira pura e sincrona, pequena,
para intencoes de selecao e navegacao do mapa. Depois que `app::input` ja
classificou pan ou zoom, ou que a rota existente de gesto primario confirmou a
selecao de State, ela produz `SelectionNavigationRequest`. O request conserva
coordenadas de tela e a intencao Ctrl; ele nunca contem ID de provincia ou
State resolvido. `Canvas::apply_selection_navigation` continua resolvendo o
pixel, mutando `StateEditSession`/selecao e movendo a camera. Nao ha fila,
controller, ownership de gesto ou historico novo nesta fronteira. Os requests
de Problems (Go-to e marcador diagnostico temporario), picker, pintura, brush,
fill e lasso continuam em suas rotas atuais.

## Fronteira de gesto de edicao primario

`app::edit_gesture` separa o ciclo de entrada do botao esquerdo da execucao da
ferramenta. Depois da classificacao pura e da captura de UI, `App` encaminha
um `MapGestureRequest::{Begin, Continue, End}` pequeno, em coordenadas de tela
e com modificadores, para `Canvas::handle_edit_gesture`. O request nao contem
pixels, IDs de provincia/State, geometria de brush/lasso ou resultado de
edicao. `Canvas` interpreta a ferramenta e o workspace atuais e continua sendo
o dono de conversao tela→mapa, hit test, mutacao, sessao e historico.

`painting` continua em `App` como estado transitorio de entrada: ele so indica
que a pressao primaria aceita deve encaminhar movimento subsequente. Ele nao
identifica ferramenta, nem decide se o movimento pinta; Canvas revalida isso em
cada `Continue`. `left_press_consumed` tambem continua em `App`: uma pressao
capturada pela UI deve consumir sua release correspondente e nunca criar um
`End` fantasma no mapa.

| Estado de entrada | Evento | Resultado |
| --- | --- | --- |
| `Idle` | pressao esquerda adiada pela UI para o mapa | `Begin`; `painting` so fica ativo se Canvas aceitar movimento |
| `Idle` | pressao capturada por UI | `left_press_consumed = true` |
| gesto primario ativo | movimento | `Continue` em coordenadas de tela |
| gesto primario ativo | release esquerda, inclusive sobre UI | `End`, depois `Idle` |
| pressao consumida | release esquerda | limpa `left_press_consumed`, sem request de mapa |
| qualquer estado | substituicao de projeto/unfocus | App limpa transitorios; Canvas anterior e descartado/cancela brush atual |

O gesto de entrada (down → move → up) nao e uma transacao de edicao. Uma
ferramenta de Canvas decide se inicia/finaliza transacao de `History` ou de
`StateEditSession`; fill e lasso de State continuam sendo acoes de clique com
suas semanticas existentes. Pan direito, picker medio e wheel permanecem fora
de `MapGestureRequest`.

| Responsabilidade | Dono |
| --- | --- |
| estado bruto do botao / captura da UI | `App` + `app::input` |
| intencao de ciclo de gesto | `app::edit_gesture` |
| `ToolMode`, conversao tela→mapa e hit test | `Canvas` |
| mutacao de provincias / Create Province | `Canvas` e edicao existente |
| brush, fill e lasso de State | `Canvas` / `StateEditSession` existente |
| historico | `History` e `StateEditSession` existentes |
| dirty, validacao e invalidacao de apresentacao | ciclo de edicao existente em `Canvas` |

| Evento bruto | Classificacao | Comando | Executor | Dono da mutacao/historico |
| --- | --- | --- | --- | --- |
| Tecla global | `app::input` | `ApplicationCommand::{Save,OpenProject,...}` | `App` / controladores existentes | Save UI/engine ou lifecycle existente |
| Tecla de mapa | `app::input` | `MapKeyboardCommand` | `App` para a API Canvas existente | `Canvas`, `StateEditSession`, `History` |
| Clique/arrasto primario | `app::input` + resultado da UI + `app::edit_gesture` | `MapGestureRequest::{Begin,Continue,End}`; State sem ferramenta ativa vira `SelectStateAt` dentro de Canvas | `Canvas::handle_edit_gesture` / `Canvas::apply_selection_navigation` | Canvas/ferramenta existente |
| Botao direito/meio | `app::input` | `BeginPan`, `EndPan`, `PickBrush`; apenas pan vira request | `Canvas::apply_selection_navigation` / ferramenta existente | `Canvas` |
| Movimento/relativo | `app::input` | `RelativeMotionCommand::PanBy` vira `PanBy` | `Canvas::apply_selection_navigation` | Canvas/camera existente |
| Roda | `app::input` apos scroll de UI | somente `WheelCommand::Zoom` vira `Zoom`; raio e captura permanecem locais | `Canvas::apply_selection_navigation` / Canvas existente | Canvas/camera existente |
| Esc de mapa | `app::input` | `MapKeyboardCommand::CancelTool`; fallback final vira `ClearStateSelection` | `Canvas::apply_selection_navigation` | Canvas/selecao existente |
| File drop | `app::input` | `FileDropCommand::OpenProject` | `ProjectLifecycleController` via App | carregador/lifecycle existente |
| Resize | `app::input` | `ViewportCommand::RebuildInterface` | `App` / `Interface` | recursos de janela/UI existentes |

Captura de Preferences, Save review, inspector picker/search e editor de
propriedades e classificada antes de atalhos ou gestos de mapa. O resultado de
`Interface::on_mouse_click` e convertido em `PrimaryClickOutcome`; somente o
resultado que a UI explicitamente adia gera um comando de gesto para o mapa.
Assim, uma pressao capturada ainda consome sua release correspondente e nao cria
click-through. O classificador e deliberadamente sem alocacao nas rotas de
cursor, movimento relativo e roda; o caminho recebido pelo file drop permanece
com o App e nao e clonado para um comando.

## Componentes reutilizados

- carregamento BMP e parser de `definition.csv`;
- resolucao de pixel e RGB, centros, limites, vizinhos e fronteiras;
- renderizacao OpenGL, camera, zoom, pan e overlays;
- selecao, lasso e atualizacao parcial de textura;
- undo/redo herdado para o modo legado;
- alertas, problemas graficos e erros recuperaveis;
- abstracao existente de diretorio e ZIP.

## Limites de projeto

### Strategic Regions (Step 9A)

`project::strategic_regions` e um dominio somente-leitura para
`map/strategicregions/*.txt`. Ele usa exclusivamente `ProjectSources` para
listar e ler os arquivos efetivos, portanto compartilha merge de diretorio,
proveniencia, `replace_path` e precedencia projeto → DLC → DLC integrada →
base. `default.map` nao configura esse diretorio. O resultado reutilizavel
mantem IDs esparsos, token `name`, nome localizado opcional, provincias na
ordem declarada, `naval_terrain`, fonte e span; falhas de arquivo tornam a
cobertura parcial sem descartar arquivos validos.

Strategic Regions sao **validados e indexados**, mas ainda **nao sao
editaveis nem Save-owned**. A validacao e o
`ProvinceReferenceIndex` consomem o mesmo resultado carregado; nao ha reparse.
Diagnosticos de ausencia global so ocorrem com cobertura completa, enquanto
contradicoes positivas (duplicidade, referencia inexistente, associacao
multipla e State dividido) continuam reportaveis com cobertura parcial.

### Strategic Regions inspector (Step 9B)

`strategic_regions_ui::StrategicRegionsController` transforma somente o
`StrategicRegionLoadResult` ja pertencente ao projeto em lista, filtro, detalhe
e requests tipados. O fluxo e `StrategicRegionLoadResult →
StrategicRegionsController → request → App/Canvas`; o controlador nao importa
`Canvas` ou `App`, nao relê fontes e nao possui dados de dominio. O painel usa
ordem crescente de ID, mostra chave de localizacao mesmo quando ha nome
resolvido, preserva a ordem das Provincias e permite navegar somente membros
existentes. Trocar a geracao limpa identidade, busca e selecao do projeto
anterior.

### Strategic Regions map view (Step 9B2)

`MapBaseView::StrategicRegions` e uma camada-base somente leitura, composta a
partir do `StrategicRegionLoadResult` ja carregado. O cache base e ligado a
geracao do projeto/mapa; a decoracao de State-split depende apenas da revisao
efetiva de Estados; e a selecao e uma camada leve de `Canvas`. A classificacao
e explicita (`Assigned`, `Ambiguous`, `Unassigned`, `Unknown`), usa mapas
esparsos e trata fontes ausentes/parciais como desconhecidas. Nenhuma fonte e
relida por frame e a funcionalidade nao adquire permissao de edicao ou Save.

Maturidade atual: **DOMAIN: YES; VALIDATION: YES; REFERENCE INDEX: YES;
INSPECTOR: YES; SOURCE NAVIGATION: YES; MAP PRESENTATION: YES; MAP SELECTION:
YES; EDITING: NO; SAVE OWNERSHIP: NO.** O painel e Project Problems podem abrir
fontes filesystem, revelar containers e copiar caminhos; entradas de archive
nao recebem uma falsa acao de abrir.

`app::project` representa a raiz de um mod. `ProjectPaths::discover` valida
`map/provinces.bmp`, `map/definition.csv` e `history/states/`; adjacencias e
rios sao opcionais.

`config.rs` separa preferências globais opcionais em
`%APPDATA%\HOI4MapEditor\config.toml` no Windows ou
`$XDG_CONFIG_HOME/HOI4MapEditor/config.toml` (fallback `~/.config`) no Linux,
de preferências opcionais do mod em `<mod>/.hoi4-map-editor/project.toml`.
Ambos usam schema versionado, staging no mesmo diretório, flush/sync, validação,
backup simples e replace atômico. O parser lossless `toml_edit` preserva
comentários, ordem e chaves desconhecidas. Ausência ou erro de configuração
nunca impede a abertura do editor e nunca marca mapas ou states como modificados.

`localization.rs` compila os catálogos UTF-8 `en-US`, `pt-BR`, `es-ES`,
`fr-FR`, `ru-RU` e `zh-CN` no binário.
Chaves de UI são resolvidas por uma API central; dados técnicos do HOI4 não
passam pela localização.

Projetos abertos pela raiz do mod editam Province Map e States sob o mesmo
coordenador. Abrir uma pasta `map/` direta ou ZIP permanece como modo legado de
Province Map, sem domínio de States.

`MapBaseView` e o estado canonico da visualizacao: `ProvinceColors`,
`ProvinceTypes`, `Terrain`, `Continents`, `Coastal`, `States` e `Political`.
`MapViewMode` e somente um alias tecnico preservado. Menus e atalhos chamam
`set_map_view_mode`; a troca nao habilita edicao geografica, nao altera
workspace, working state, dirty, historico ou ferramentas estaduais ativas.

`MapLayerState` mantem overlays independentes: Resources, imagem e opacidade,
rivers, adjacencies, IDs, fronteiras de provincia e state, labels e diagnostico
de desenvolvedor. `ViewMode` continua sendo o renderer/ferramenta tecnico
legado do mapa de provincias; `Adjacencies` nesse enum seleciona a ferramenta
de edicao, enquanto sua exibicao e controlada pelo overlay canonico.

## Leitura de estados

`app::state` contem a camada de leitura de PDXScript:

- `syntax.rs`: `SourceText`, lexer lossless, spans UTF-8 em bytes, arvore
  generica ordenada e parser com recuperacao de erros;
- `extractor.rs`: converte o bloco `state` em `StateData` sem descartar a
  arvore original ou campos desconhecidos;
- `loader.rs`: enumera somente arquivos `.txt` diretamente em
  `history/states/`, ordena os caminhos e continua apos falhas individuais;
- `model.rs`: mantem o documento sintatico, dados tipados, diagnosticos, bytes
  originais exatos e a indicacao de UTF-8 lossless.

Comentarios, whitespace e newlines permanecem na sequencia de tokens. A arvore
preserva ordem, chaves repetidas, listas posicionais e blocos desconhecidos.

`app::project::indexes` constroi `states_by_id`, `state_by_province`,
`ambiguous_provinces` e o conjunto de provincias terrestres sem estado. IDs
duplicados e atribuicoes duplicadas seguem uma politica deterministica: o
primeiro documento ordenado permanece no indice, os demais continuam carregados
e recebem diagnosticos.

## Visualizacao de estados

`app::project::view` resolve cada pixel geografico para uma classificacao de
estado e gera uma textura imutavel em memoria. Cores de estado derivam do ID;
vermelho, magenta e laranja ficam reservados para diagnosticos. Bordas entre
estados sao calculadas no carregamento, e o overlay do estado selecionado e
reconstruido somente quando a selecao muda.

Em `MapBaseView::States`, o `Canvas` usa o working set quando existe sessao de
edicao. Ctrl+click alterna a selecao de provincias terrestres editaveis; clique
normal escolhe o estado alvo; Move, Unassign, Undo, Redo e Discard atualizam
somente estruturas em memoria e regeneram textura, fronteiras, contadores e
diagnosticos visuais a partir do working set.

`MapBaseView::Political` deriva a cor do owner efetivo. Quando a metadata de
cor do pais nao esta disponivel, uma cor deterministica por tag e usada sem
colidir com as cores reservadas de diagnostico. O Image Overlay aceita BMP,
PNG e JPEG somente para leitura, exige as dimensoes exatas do mapa e e composto
abaixo de selecao, fronteiras e diagnosticos. `map/heightmap.bmp` e apenas uma
fonte automatica desse overlay generico.

## Edicao em memoria

`app::project::edit` mantem uma sessao transacional em memoria. A sessao tem um
baseline carregado do projeto e um working set separado com
`state_by_province`, `provinces_by_state`, `victory_points`,
`province_buildings`, `EditableStateProperties`, provincias terrestres sem
estado, selecao explicita de provincias e estado alvo.

`StateEditCommand` contem reassociacoes, `UpdateStateProperties`,
`UpdateProvinceData`, Create e Remove. Move, Unassign, Apply, Create e Remove
compartilham uma unica ordem de undo/redo. O modelo nao conhece serializer e
nao escreve em disco.

`app::project::properties` contem drafts temporarios de estado e provincia.
Digitacao fica no draft e nao muda working data, documentos, dirty state ou
mapa. Apply valido cria um comando atomico; Apply invalido e no-op nao entram
no historico.

## Lasso, Brush e Fill de estado

`app::project::lasso` implementa selecao por poligono sem pintar pixels:

```text
screen coordinates
-> map coordinates
-> polygon
-> clamped bounding box
-> unique province IDs
-> land/ambiguity/valid-state classification
-> cached preview
-> confirmed selected_provinces
```

Replace, Add e Remove alteram apenas a selecao confirmada. A previa, sua
confirmacao e seu cancelamento nao mudam o working set, nao marcam dirty e nao
entram no historico.

`app::project::brush` implementa State Brush separado da pintura geografica:

```text
mouse positions
-> map coordinates
-> segment sampling
-> province IDs
-> classification
-> preview boundaries
-> ReassignProvinces on release
-> working state
-> selective visual refresh
```

Cada stroke guarda IDs de provincia visitados, mostra previa durante o arrasto
e aplica no mouse release com uma unica chamada a `StateEditSession`.

`app::project::state_fill` usa a adjacencia de fronteiras ja calculada para
planejar Hovered Province, Connected Same State, Connected Unassigned e Whole
Source State. A previa e pura e nao marca dirty; Enter aplica todos os IDs
validos em uma unica transacao `ReassignProvinces`; Esc cancela.

## Patch Preview e validacao

`app::project::patch` compara baseline e working set, resolve proveniencia na
arvore sintatica e produz operacoes `Replace`, `Insert` e `Delete` com spans de
bytes e bytes esperados. Arquivos carregados nunca passam pelo renderer
canonico; bytes fora dos spans permanecem preservados. Estados criados usam
renderer canonico somente em memoria. Estados removidos geram plano de remocao.

`app::project::validation` aplica planos somente em um workspace temporario
controlado, recarrega o candidato pelos loaders reais e compara semantica,
indices, cobertura, diagnosticos estruturais e bytes. O resultado pode ser
`Passed`, `PassedWithReview`, `Failed` ou `Cancelled`. `ReviewRequired` exige
acao explicita; somente o fluxo Validate and Continue pode usar um
`PassedWithReview` atual para autorizar Save.

## Salvamento transacional

`app::project::save` e a fronteira coordenada que pode persistir arquivos do
projeto. O gate exige candidatos atuais de mapa e/ou estados, sem operacoes
`Blocked`, e um relatorio de validacao atual (`Passed`, ou
`PassedWithReview` com autorizacao explicita), alem de digests
correspondentes, fontes ainda identicas, nenhuma diferenca liquida vazia,
nenhum draft ou gesto ativo e nenhuma transacao/recovery pendente.

Depois da confirmacao explicita, a transacao segue:

```text
exclusive save.lock
-> durable journal
-> source revalidation
-> physical backup + deterministic manifest
-> backup byte verification
-> same-directory stage files
-> staged byte verification
-> second source revalidation
-> deterministic staged commit
-> real project reload
-> semantic/index/coverage/VP/building/diagnostic/byte/map comparison
-> new baseline or verified rollback
```

Metadados ficam em `<mod>/.hoi4-state-editor/`. Backups usam copias fisicas.
Stages e rollbacks ficam ao lado do destino com sufixos `.hse-stage-<id>` e
`.hse-rollback-<id>` depois de `.txt`.
O fluxo e journaled, coordenado e capaz de rollback, mas nao promete
atomicidade filesystem de projeto inteiro entre todos os arquivos e plataformas.

### Limites de durabilidade do Save Project

Tres garantias distintas sao mantidas separadas. A **correcao transacional da
aplicacao** vem de candidatos validados, ordem deterministica, backups verificados,
journal, rollback e recovery. A **atomicidade de namespace por arquivo** vem da
substituicao especifica da plataforma; stages e rollbacks sao siblings do destino,
portanto o `rename` Unix nao depende de cruzar filesystems. A **durabilidade contra
queda de energia** exige barreiras adicionais: os arquivos novos/staged, backups,
locks e journals usam `sync_all`; no Unix o diretorio-pai tambem e sincronizado
depois de criar, publicar, renomear ou remover entradas que fazem parte do protocolo.

Essas barreiras pressupõem um filesystem local com semantica POSIX normal de
`rename` e `fsync` (por exemplo ext4, btrfs ou XFS configurados pelo sistema). Elas
nao prometem seguranca universal em filesystems de rede, FUSE ou montagens com
semantica incomum. Windows conserva `ReplaceFileW`/`MoveFileExW`; nao tenta abrir
diretorios como arquivos comuns.

A ordem de commit e deterministica: substituicoes de arquivos existentes por
caminho relativo normalizado, depois criacoes por caminho e, por ultimo,
remocoes por caminho. Todo destino ja possui backup verificado antes dessa
sequencia; rollback percorre a mesma lista em ordem inversa e verifica os bytes
originais.

## State Inspector e catalogos

`app::inspector` mantem somente estado de apresentacao da sessao: visibilidade,
secao, scroll, busca e nivel de diagnosticos. `InspectorLayout`
separa toolbar, sidebar, `MapViewport` e painel lateral. `Interface` publica
esse viewport unico; camera, picking, zoom, lasso, brush, labels, tooltip e
hit-testing passam por ele. A textura pode existir sob o painel, mas eventos do
Inspector nunca chegam ao mapa.

O Inspector nao criou um segundo modelo de edicao. Seus controles abrem e
alteram `StatePropertyDraft` e `ProvinceDataDraft`; Apply continua chamando os
comandos `UpdateStateProperties` e `UpdateProvinceData`. Troca de estado, Undo,
fechamento e Save continuam protegidos pelas regras existentes de draft
pendente.

`app::project::catalog` constroi um `GameDefinitionCatalog` deterministico no
carregamento do projeto. A precedencia combina fallbacks embutidos, base game
opcional, mod carregado e valores observados. Resources, state categories,
buildings e country tags guardam a origem da definicao. Pastas ausentes e
arquivos invalidos geram diagnosticos nao fatais; valores customizados do
estado atual continuam editaveis.

O arquivo-fonte e resolvido por `WorkingStateOrigin`. Estados carregados podem
ser abertos por duplo clique ou pelo cabecalho; estados criados informam que
ainda nao possuem arquivo. O pedido externo carrega um path, nunca uma linha de
shell concatenada, e o dispatcher aceita opener injetavel para testes headless.

## Fluxo de dados

```text
state file
-> SourceText
-> tokens
-> syntax tree
-> StateData
-> states_by_id / state_by_province
-> diagnostics and StateLoadSummary
-> cached state texture / state selection
-> in-memory state edit baseline and working set
-> temporary validated property draft
-> unified province/property/lifecycle edit history
-> refreshed state texture / selection overlays when geography changed
-> semantic diff / syntax provenance
-> in-memory patch plan / parsed textual preview
-> isolated temporary candidate / real project reload
-> semantic, index, diagnostic and byte comparison report
-> exact Passed authorization
-> backup / staging / journal / deterministic commit
-> post-save reload / new baseline or verified rollback
```

O mapa continua seguindo `provinces.bmp -> RGB -> definition.csv -> province
ID`. Depois que o `Bundle` geografico e carregado, `Canvas::load_project`
fornece os IDs reais ao carregador de estados.

## Limites atuais

- Lasso de selecao e State Brush operam por provincia; nao ha pintura de
  pixels, merge, split ou brush com raio para estados.
- `Ctrl+S` usa Save Project para salvar candidatos de mapa e estados quando a
  implementacao unificada esta habilitada. Exports de Province criam copias e
  nao limpam o dirty state.
- Blocos datados sao detectados e preservados, nao interpretados.
- Arquivos que nao sao UTF-8 recebem diagnostico e representacao lossy somente
  para inspecao; nunca sao reescritos.
- A camada geografica aceita IDs externos positivos esparsos em `definition.csv`;
  gaps sao preservados e nunca compactados implicitamente.
- O Save Project desta fase altera somente `provinces.bmp`, `definition.csv` e
  `history/states/*.txt`, alem de `adjacencies.csv` quando o candidato de mapa o
  contem; rivers continuam fora do coordenador.
- Sem suporte a ZIP de mod; ZIP permanece apenas no modo legado.

## Riscos conhecidos

- O modo legado ainda contem invariantes internas com `expect`.
- Mapas grandes e entradas ZIP ainda sao carregados integralmente em memoria.
- Strings com escapes permanecem preservadas na arvore; a extracao tipada
  remove apenas as aspas externas e ainda nao interpreta todos os escapes.
