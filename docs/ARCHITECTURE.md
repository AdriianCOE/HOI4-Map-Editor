# Arquitetura do HOI4 Map Editor

HOI4 Map Editor e o nome publico do projeto. O crate/binario
`hoi4_state_editor`, o diretorio de backup `.hoi4-state-editor`, `MapViewMode`
e `ViewMode` permanecem como nomes tecnicos de compatibilidade; nao representam
uma segunda marca publica.

## Base herdada

O projeto e um fork do HOI4 Province Editor de ScottyThePilot. A inicializacao
continua em `main.rs`; `events.rs` executa o loop Piston; `App` traduz entradas
em acoes; `Canvas` reune sessao visual, camera, ferramentas e texturas;
`Bundle`, `Map` e `History` mantem os dados geograficos e undo/redo; e
`map/bridge.rs` usa `util/files.rs` para ler pastas ou ZIPs.

## Componentes reutilizados

- carregamento BMP e parser de `definition.csv`;
- resolucao de pixel e RGB, centros, limites, vizinhos e fronteiras;
- renderizacao OpenGL, camera, zoom, pan e overlays;
- selecao, lasso e atualizacao parcial de textura;
- undo/redo herdado para o modo legado;
- alertas, problemas graficos e erros recuperaveis;
- abstracao existente de diretorio e ZIP.

## Limites de projeto

`app::project` representa a raiz de um mod. `ProjectPaths::discover` valida
`map/provinces.bmp`, `map/definition.csv` e `history/states/`; adjacencias e
rios sao opcionais.

`Canvas::MapAccessMode` separa dois fluxos:

- `ReadOnly`: projeto de estado aberto pela raiz do mod. Os arquivos
  geograficos sao base visual e nao sao salvos pelo fluxo de estados.
- `EditableProvinceMap`: modo legado para abrir uma pasta `map/` direta ou ZIP
  de mapa. Mantem temporariamente as ferramentas herdadas do Province Editor.

`MapBaseView` e o estado canonico da visualizacao: `ProvinceColors`,
`ProvinceTypes`, `Terrain`, `Continents`, `Coastal`, `States` e `Political`.
`MapViewMode` e somente um alias tecnico preservado. Menus e atalhos chamam
`set_map_view_mode`; a troca nao habilita edicao geografica, nao altera
workspace, working state, dirty, historico ou ferramentas estaduais ativas.

`MapLayerState` mantem overlays independentes: imagem e opacidade, rivers,
adjacencies, IDs, fronteiras de provincia e state, labels e diagnostico de
desenvolvedor. `ViewMode` continua sendo o renderer/ferramenta tecnico legado
do mapa de provincias; `Adjacencies` nesse enum seleciona a ferramenta de
edicao, enquanto sua exibicao e controlada pelo overlay canonico.

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
acao explicita e nunca autoriza Save.

## Salvamento transacional

`app::project::save` e a unica fronteira que pode persistir arquivos de estado.
O gate exige um `ProjectPatchPlan` atual e integralmente `Safe`, um
`RoundTripValidationReport` atual com status exatamente `Passed`, digests
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
-> deterministic rename commit
-> real project reload
-> semantic/index/coverage/VP/building/diagnostic/byte/map comparison
-> new baseline or verified rollback
```

Metadados ficam em `<mod>/.hoi4-state-editor/`. Backups usam copias fisicas.
Stages e rollbacks ficam ao lado do destino com sufixos `.hse-stage-<id>` e
`.hse-rollback-<id>` depois de `.txt`.

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

- Fases 0 a 4C estao implementadas: parser, leitura, visualizacao, selecao,
  reassociacao, preview lossless, validacao round-trip, backup e salvamento
  transacional.
- Fase 5A/5A.1 esta em andamento: Inspector, catalogos, pickers, toolbar de
  states, State Fill, Political, heightmap, labels, diagnosticos e branding
  existem, mas o acabamento de UX ainda nao esta finalizado.
- Lasso de selecao e State Brush operam por provincia; nao ha pintura de
  pixels, merge, split ou brush com raio para estados.
- `Ctrl+S` executa State Save somente quando elegivel; `Save As`, autosave,
  retencao avancada, integracao Git e execucao do HOI4 continuam fora do
  contrato.
- Blocos datados sao detectados e preservados, nao interpretados.
- Arquivos que nao sao UTF-8 recebem diagnostico e representacao lossy somente
  para inspecao; nunca sao reescritos.
- A camada geografica ainda exige IDs contiguos em `definition.csv`.
- Sem alteracao de `provinces.bmp`, `definition.csv`, `adjacencies.csv` ou
  `rivers.bmp` quando uma raiz de mod e aberta.
- Sem suporte a ZIP de mod; ZIP permanece apenas no modo legado.

## Riscos conhecidos

- O modo legado ainda contem invariantes internas com `expect`.
- Mapas grandes e entradas ZIP ainda sao carregados integralmente em memoria.
- O arquivo legado `hoi4pe_config.toml` mantem esse nome por compatibilidade.
- Strings com escapes permanecem preservadas na arvore; a extracao tipada
  remove apenas as aspas externas e ainda nao interpreta todos os escapes.
