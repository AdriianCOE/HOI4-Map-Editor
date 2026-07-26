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
- `model.rs`: mantém o documento sintático, dados tipados, diagnósticos e o
  texto original dentro de `PdxDocument::source`.

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

Após um comando, o Canvas combina os bounds geográficos das províncias
alteradas. Até 128 províncias cobrindo no máximo 25% do mapa usam atualização
seletiva da textura e das bordas naquela região; lotes maiores usam o rebuild
completo existente. A política é deliberadamente simples e ambos os caminhos
são registrados no console.

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
```

O mapa continua seguindo `provinces.bmp -> RGB -> definition.csv -> province
ID`. Depois que o `Bundle` geográfico é carregado, `Canvas::load_project`
fornece os IDs reais ao carregador de estados. Assim, `definition.csv` não é
relido e referências desconhecidas podem ser diagnosticadas com segurança.

## Componentes a substituir ou adaptar

- o salvamento futuro alterará somente documentos de estado afetados;
- controles herdados de edição geográfica poderão ser removidos após existir
  substituição funcional.

## Limites das Fases 2A, 2B, 3A, 3B e 3C

- parser, leitura, visualização, seleção e reassociação em memória
  implementados, mas sem serializer, diff, backup ou salvamento;
- lasso de seleção por província implementado, mas sem brush, pintura,
  merge ou split de estados;
- criação e remoção controladas existem somente no working set; nenhum arquivo
  é criado, removido, renomeado ou associado a um nome final;
- propriedades gerais, owner/controller, cores, claims, recursos, construções
  estaduais, victory points e construções provinciais podem ser editados
  somente no working set; IDs, sintaxe e caminho do arquivo permanecem somente
  leitura;
- `victory_points` e `province_buildings` acompanhando a província movida são
  realocados no working set quando não há conflito; sintaxe desconhecida segue
  preservada no documento original;
- `Ctrl+S` e `Save As` continuam bloqueados em projetos de estado;
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
