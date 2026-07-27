# Arquitetura do HOI4 State Editor

## Base herdada

O projeto é um fork do HOI4 Province Editor de ScottyThePilot. A inicialização
continua em `main.rs`; `events.rs` executa o loop Piston; `App` traduz entradas
em ações; `Canvas` reúne sessão visual, câmera, ferramentas e texturas;
`Bundle`, `Map` e `History` mantêm os dados geográficos e undo/redo; e
`map/bridge.rs` usa `util/files.rs` para ler pastas ou ZIPs.

## Componentes reutilizados

- carregamento BMP e parser de `definition.csv`;
- resolução de pixel e RGB, centros, limites, vizinhos e fronteiras;
- renderização OpenGL, câmera, zoom, pan e overlays;
- seleção, lasso e atualização parcial de textura;
- undo/redo herdado para o modo legado;
- alertas, problemas gráficos e erros recuperáveis;
- abstração existente de diretório e ZIP.

## Novos limites

`app::project` representa a raiz de um mod. `ProjectPaths::discover` valida
`map/provinces.bmp`, `map/definition.csv` e `history/states/`; adjacências e
rios são opcionais. `Hoi4Project` é o ponto inicial para dados que serão
carregados nas próximas fases.

`app::state` contém a camada de leitura de PDXScript:

- `syntax.rs`: `SourceText`, lexer lossless, spans UTF-8 em bytes, árvore
  genérica ordenada e parser com recuperação de erros;
- `extractor.rs`: converte o bloco `state` em `StateData` sem descartar a
  árvore original ou campos desconhecidos;
- `loader.rs`: enumera somente arquivos `.txt` diretamente em
  `history/states/`, ordena os caminhos e continua após falhas individuais;
- `model.rs`: mantém o documento sintático, dados tipados, diagnósticos, bytes
  originais exatos e a indicação de UTF-8 lossless.

Comentários, whitespace e newlines permanecem na sequência de tokens. A
árvore preserva ordem, chaves repetidas, listas posicionais e blocos
desconhecidos. Não há serializer nem salvamento de PDXScript.

`app::project::indexes` constrói `states_by_id`, `state_by_province` e
`ambiguous_provinces`, além do conjunto de províncias terrestres sem estado.
IDs duplicados e atribuições duplicadas seguem uma política determinística: o
primeiro documento ordenado permanece no índice, os demais continuam
carregados e recebem diagnósticos.

`app::project::view` resolve cada pixel geográfico para uma classificação de
estado e gera uma textura imutável em memória. Cores de estado são derivadas
do ID sem dependência aleatória; vermelho, magenta e laranja são reservados
para diagnósticos. As bordas são calculadas uma vez durante o carregamento e
o overlay do estado selecionado é reconstruído somente quando a seleção
muda.

`app::project::edit` mantém uma sessão transacional em memória.
A sessão tem um baseline carregado do projeto e um working set separado com
`state_by_province`, `provinces_by_state`, `victory_points`,
`province_buildings`, `EditableStateProperties`, províncias terrestres sem
estado, seleção explícita de províncias e estado alvo. `StateEditCommand`
contém tanto reassociações quanto `UpdateStateProperties`, portanto Move,
Unassign e Apply compartilham uma única ordem de undo/redo. O modelo não
conhece serializer e não escreve em disco.

Na Fase 3C, o mesmo working set mantém a origem (`Loaded` ou
`CreatedInSession`) e o ciclo de vida (`Active` ou `RemovedInSession`) de cada
estado. IDs vindos de documentos válidos, documentos inválidos com prefixo
numérico, estados ativos e comandos Create no undo/redo formam um único
conjunto reservado. Create e Remove são comandos atômicos no histórico
existente; não existe um segundo sistema de transações.

Create adiciona propriedades e, opcionalmente, transfere a seleção atual por
`ProvinceEditDelta`. Remove captura um snapshot completo e exige `MoveToState`
ou `Unassign`; undo restaura propriedades, ordem de victory points,
construções provinciais e associações. Um estado carregado removido permanece
como tombstone somente na sessão. Nenhuma dessas operações cria, apaga,
renomeia ou escreve um arquivo.

`app::project::properties` contém os campos editáveis das Fases 3A e 3B e os
drafts textuais temporários de estado e província. A separação é:

```text
baseline imutável
-> working set confirmado
-> draft temporário da Canvas
-> validação completa
-> um UpdateStateProperties
-> histórico unificado
```

Digitação, inclusive valores numéricos incompletos, fica no draft e não muda
working data, documentos, dirty state ou mapa. Identificadores permanecem
abertos a valores customizados; coleções usam representação determinística e
rejeitam chaves vazias ou duplicadas. Apply inválido é atômico e no-op não cria
comando.

O draft provincial contém somente victory point e construções da província
ativa. `UpdateProvinceData` confirma os dois conjuntos como um comando, sem
alterar textura ou fronteiras. Move e Unassign reutilizam a transferência
transacional existente, inclusive enquanto os dados ficam temporariamente
desassociados.

`Canvas::MapAccessMode` continua separando dois fluxos de acesso:

- projeto de estado: `ReadOnly`, usa a raiz do mod e nunca salva nem altera os
  arquivos geográficos;
- mapa legado: `EditableProvinceMap`, preserva temporariamente as ferramentas
  do Province Editor para compatibilidade.

`MapViewMode` é independente desse acesso e alterna entre `Provinces` e
`States`. Trocar a visualização não habilita edição, não altera a textura
geográfica e não registra operações no histórico.

Em `MapViewMode::States`, o `Canvas` usa o working set quando existe sessão de
edição. Ctrl+click alterna a seleção de províncias terrestres editáveis;
clique normal escolhe o estado alvo; os comandos Move, Unassign, Undo, Redo e
Discard atualizam somente estruturas em memória e regeneram a textura,
fronteiras, contadores e diagnósticos visuais a partir do working set.
O menu Edit também pode selecionar em lote todas as províncias terrestres do
estado alvo sem criar uma operação no histórico.

`app::project::lasso` implementa a seleção avançada da Fase 2B sem reutilizar
a parte destrutiva do lasso geográfico. O fluxo é:

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

`StateLassoPhase` separa `Inactive`, `Drawing` e `Preview`. Durante `Drawing`,
somente os pontos em coordenadas do mapa são mantidos e reprojetados pela
câmera; zoom e pan não alteram o polígono. Ao fechar, o bounding box é
percorrido uma vez e a prévia guarda IDs, nunca pixels. `CentroidInside` é o
critério padrão; `AnyIntersection` e `MajorityInside` reutilizam
`ProvinceData::pixel_count` para evitar uma segunda varredura global.

Replace, Add e Remove alteram apenas a seleção confirmada da sessão. A prévia,
sua confirmação e seu cancelamento não mudam o working set, não marcam dirty e
não entram no histórico. Províncias marítimas, lagos, ID zero e pixels
desconhecidos são ignorados. Províncias ambíguas ou ligadas a estados inválidos
ficam bloqueadas e visualmente separadas.

Move e Unassign continuam chamando exclusivamente o comando transacional da
Fase 2A. Um lote, independentemente da origem da seleção, produz no máximo uma
entrada de undo e mantém preflight, rollback, victory points, construções
provinciais e índices no mesmo ponto de autoridade.

`app::project::brush` implementa o State Brush da Fase 3D como uma camada
separada das ferramentas destrutivas de pintura geográfica. O fluxo é:

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

O brush guarda somente IDs de província visitados. Durante o arrasto ele
classifica novos IDs como editáveis, no-op, ignorados ou bloqueados, atualiza
apenas contornos de prévia e não altera o working set. O mouse release filtra
os editáveis e chama `StateEditSession::reassign_provinces` uma vez, com
`Some(target_state_id)` no modo Assign e `None` no modo Unassign. Assim, victory
points, construções provinciais, conflitos, dirty state, Undo e Redo continuam
centralizados no comando existente.

Após um comando, o Canvas combina os bounds geográficos das províncias
alteradas. Até 128 províncias cobrindo no máximo 25% do mapa usam atualização
seletiva da textura e das bordas naquela região; lotes maiores usam o rebuild
completo existente. A política é deliberadamente simples e ambos os caminhos
são registrados no console.

## Planejamento lossless da Fase 4A

`app::project::patch` compara o baseline com o working set atual, resolve a
proveniência diretamente na árvore sintática e produz operações `Replace`,
`Insert` e `Delete` com spans de bytes e bytes esperados. As operações são
verificadas contra overlaps, aplicadas em ordem decrescente somente sobre uma
cópia de `original_bytes` e o resultado é novamente parseado e comparado
semanticamente.

Arquivos carregados nunca passam pelo renderer canônico. Comentários, campos
desconhecidos, ordem, whitespace, BOM e line endings fora dos spans permanecem
byte a byte. Fragmentos provinciais transferidos podem reutilizar os bytes da
origem; quando isso não pode ser provado, o arquivo fica `Blocked`. UTF-8
lossy, fonte alterada externamente, binding autoritativo duplicado, histórico
datado ambíguo e patch sobreposto também bloqueiam.

Estados criados usam renderer canônico somente em memória, com estilo
predominante do projeto e nome determinístico
`history/states/{id}-State_{id}.txt`. Estados carregados removidos geram apenas
`PlannedFileRemoval`. O `ProjectPatchPlan` guarda fingerprints, previews,
diffs, diagnósticos e uma geração ligada à revisão do working set; mudanças,
Undo/Redo e Discard tornam o preview anterior stale.

## Validação isolada da Fase 4B

`app::project::validation` recebe um `ProjectPatchPlan` atual e cria um
workspace descartável em `%TEMP%/hoi4-state-editor/roundtrip`. O candidato
recebe cópias reais de `provinces.bmp`, `definition.csv` e dos arquivos
diretos `history/states/*.txt`; hardlinks, symlinks e caminhos fora desse
conjunto não fazem parte do fluxo. Um resolvedor central rejeita caminhos
absolutos, prefixos de drive, `..`, extensões inesperadas, colisões sem
distinção de caixa e sobreposição entre criação, modificação e remoção.

Antes da cópia, fingerprints e bytes autoritativos são relidos da origem.
Somente o candidato recebe as operações da Fase 4A. Em seguida, o mesmo
`Bundle::load` geográfico e o carregador normal de estados recarregam o
workspace. A comparação cobre estados ativos e preservados, propriedades,
victory points, construções, `states_by_id`, `state_by_province`, províncias
sem estado, ambiguidades, cobertura e novos diagnósticos estruturais. Arquivos
inalterados precisam continuar idênticos byte a byte; arquivos modificados
precisam coincidir exatamente com o preview; criações e remoções precisam
existir somente no candidato.

O resultado é `Passed`, `PassedWithReview`, `Failed` ou `Cancelled`.
`ReviewRequired` exige uma ação explícita e nunca produz `Passed`. Planos stale
ou `Blocked`, fonte alterada e caminhos inseguros falham antes do workspace.
Por padrão, pass, falha e cancelamento removem o diretório temporário; uma
política diagnóstica explícita pode reter somente falhas e sempre registra o
caminho. A origem é verificada novamente depois das comparações. Esta camada
continua isolada e não escreve no mod.

## Salvamento transacional da Fase 4C

`app::project::save` é a única fronteira que pode persistir arquivos de
estado. O gate central exige um `ProjectPatchPlan` atual e integralmente
`Safe`, um `RoundTripValidationReport` atual com status exatamente `Passed`,
digests correspondentes, fontes ainda idênticas, nenhuma diferença líquida
vazia, nenhum draft ou gesto ativo e nenhuma transação/recovery pendente.
`PassedWithReview` nunca autoriza Save.

Depois da confirmação explícita, a transação:

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

Metadados ficam em `<mod>/.hoi4-state-editor/`. Backups usam cópias físicas,
nunca hardlinks; arquivos criados constam no manifesto sem bytes anteriores.
Stages e rollbacks ficam ao lado do destino com sufixos
`.hse-stage-<id>` e `.hse-rollback-<id>` depois de `.txt`, garantindo a mesma
filesystem boundary para rename e evitando que HOI4 os carregue como states.

Modified renomeia o original para rollback e o stage para o path final.
Created renomeia somente o stage, depois de confirmar que o destino continua
ausente. Removed renomeia o original para rollback; não há delete antes de
backup. O journal é atualizado por arquivo. Cancelamento é aceito somente
antes de `Committing`.

Rollback percorre as operações registradas em ordem inversa, verifica bytes
antes de remover um candidato e restaura modified/removed ou elimina created.
Falha parcial mantém lock, journal, backup e paths de rollback para recuperação
manual. Um lock encontrado no carregamento bloqueia edição e novo Save até a
ação explícita de recovery executar rollback verificado. Backup e relatório
permanecem depois de sucesso; stages e rollbacks são removidos somente depois
da validação pós-save.

No sucesso, o projeto salvo é recarregado pelo loader real e substitui
`Hoi4Project` e `StateEditSession`: o working set passa a baseline, Undo/Redo e
dirty ficam vazios, e seleção/target são preservados apenas quando ainda
válidos. `Save As` continua fora desse contrato e o Save geográfico legado
permanece separado.

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
ID`. Depois que o `Bundle` geográfico é carregado, `Canvas::load_project`
fornece os IDs reais ao carregador de estados. Assim, `definition.csv` não é
relido e referências desconhecidas podem ser diagnosticadas com segurança.

## Componentes a substituir ou adaptar

- o salvamento de estados altera somente documentos afetados por plano Safe;
- controles herdados de edição geográfica poderão ser removidos após existir
  substituição funcional.

## Limites das Fases 2A a 4C

- parser, leitura, visualização, seleção, reassociação, preview lossless,
  validação round-trip, backup e salvamento transacional implementados;
- lasso de seleção e State Brush por província implementados, mas sem pintura
  de pixels, merge, split ou brush com raio;
- criação e remoção controladas existem no working set e só alcançam paths
  reais por um plano Safe validado e Save explicitamente confirmado;
- propriedades gerais, owner/controller, cores, claims, recursos, construções
  estaduais, victory points e construções provinciais podem ser editados
  primeiro no working set e persistidos somente após preview e validação;
  IDs existentes e paths continuam protegidos pelo plano;
- `victory_points` e `province_buildings` acompanhando a província movida são
  realocados no working set quando não há conflito; sintaxe desconhecida segue
  preservada no documento original;
- `Ctrl+S` executa State Save somente quando elegível; `Save As`, autosave,
  retenção avançada e restauração gráfica de backups não existem;
- blocos datados são detectados e preservados, não interpretados;
- arquivos que não são UTF-8 recebem diagnóstico e uma representação lossy
  somente para inspeção; nunca são reescritos;
- a camada geográfica ainda exige IDs contíguos em `definition.csv`;
- sem alteração de `provinces.bmp`, `definition.csv`, `adjacencies.csv` ou
  `rivers.bmp` quando uma raiz de mod é aberta;
- sem suporte a ZIP de mod; ZIP permanece apenas no modo legado.

## Riscos conhecidos

- o modo legado ainda contém invariantes internas com `expect`;
- mapas grandes e entradas ZIP ainda são carregados integralmente em memória;
- o arquivo legado `hoi4pe_config.toml` mantém esse nome por compatibilidade;
- strings com escapes permanecem preservadas na árvore; a extração tipada
  remove apenas as aspas externas e ainda não interpreta todos os escapes.
