# Plano de migracao do HOI4 Map Editor

HOI4 Map Editor e o nome publico. `hoi4_state_editor`, `.hoi4-state-editor`,
`MapViewMode` e outros nomes internos permanecem por compatibilidade ate uma
eventual migracao tecnica separada.

## Fase 0 - preparar e estabilizar a base

Identificar o fork, documentar a arquitetura herdada, validar a raiz do mod,
modelar estados sem parser e tornar o mapa somente leitura em projetos de
estado.

Status: concluida.

## Fase 1A - carregar projeto e indexar estados

Enumeracao deterministica, lexer e parser genericos, arvore com spans e trivia,
extracao tipada, `states_by_id`, `state_by_province`, `ambiguous_provinces`,
diagnosticos e resumo de carregamento foram implementados. Arquivos invalidos
nao interrompem os demais.

Status: concluida.

## Fase 1B - visualizacao e selecao de estados

Resolver `pixel -> RGB -> province ID -> state ID` e gerar uma textura de
estados sem alterar o bitmap geografico. A visualizacao usa cores
deterministicas por ID, bordas entre estados e cores reservadas para provincias
ambiguas, terrestres sem estado e referencias provinciais inexistentes. O
clique seleciona o estado inteiro e mostra informacoes basicas somente para
leitura.

Status: concluida.

## Fase 2A - edicao transacional em memoria

Reutilizar picking para mudar somente associacoes entre provincias terrestres e
estados existentes em uma sessao temporaria. A implementacao atual mantem
baseline e working set separados, historico proprio de undo/redo, descarte
explicito, prompt de descarte ao fechar/trocar projeto, bloqueio de `Ctrl+S`,
selecao por Ctrl+click, estado alvo por clique normal, Move, Unassign, selecao
em lote das provincias do alvo e atualizacao da textura/contadores/diagnosticos
a partir do working set.

Status: concluida.

## Fase 2B - selecao avancada e UX de edicao

O lasso especifico de estados captura um poligono em coordenadas do mapa,
calcula uma previa por bounding box e seleciona provincias terrestres inteiras.
Replace, Add e Remove controlam a combinacao com a selecao atual.
CentroidInside e o padrao; AnyIntersection e MajorityInside ficam disponiveis
no menu da ferramenta. Mar, lago, IDs desconhecidos, ambiguidades e estados
invalidos nao entram silenciosamente na selecao editavel.

Confirmar a previa altera somente `selected_provinces`. Move e Unassign
reutilizam o comando transacional da Fase 2A, portanto o lote e atomico e gera
uma unica entrada de undo. Regioes pequenas recebem atualizacao visual seletiva;
regioes grandes usam rebuild completo. Nada e serializado.

Status: concluida.

## Fase 3A - propriedades de estado em memoria

Nome logico, manpower, category, owner/controller, cores, claims, recursos e
construcoes estaduais sao editados por draft validado e um comando atomico.

Status: concluida.

## Fase 3B - dados provinciais em memoria

Victory points e construcoes provinciais sao editados no estado ativo e
acompanham a provincia em Move, Unassign, Undo, Redo e Discard.

Status: concluida.

## Fase 3C - criacao e remocao controlada em memoria

Criar estado vazio ou a partir da selecao e remover estado com Move All ou
Unassign All usam o mesmo working set e historico transacional. IDs ocupados ou
reservados sao rejeitados, estados carregados removidos viram tombstones da
sessao e snapshots completos tornam Undo/Redo reversiveis. Nenhum arquivo de
estado e criado, apagado, renomeado ou escrito.

Status: concluida.

## Fase 3D - State Brush em memoria

Atribui ou desassocia provincias diretamente com um brush separado da pintura
geografica. O stroke amostra segmentos entre eventos do cursor, deduplica IDs,
mostra previa e aplica somente no mouse release, chamando uma unica transacao
`ReassignProvinces`. Victory points e construcoes provinciais acompanham as
provincias via o comando existente. Undo/Redo e Discard continuam em memoria,
sem serializer ou escrita em disco.

Status: concluida.

## Fase 4A - arquitetura e preview de patches lossless

Compara baseline e working state, resolve proveniencia nos tokens e spans,
valida bytes esperados e overlaps, aplica patches somente em copias na memoria
e apresenta resumo semantico, diagnosticos e diff. Bytes nao relacionados,
comentarios, formatacao, BOM, line endings, campos desconhecidos e blocos
datados permanecem preservados; casos ambiguos ficam ReviewRequired ou Blocked.
Estados novos recebem preview canonico e estados removidos apenas um plano de
remocao. Nenhum arquivo real e escrito.

Status: concluida.

## Fase 4B - validacao isolada dos planos

Aplica planos somente em copias reais dentro de um workspace temporario
controlado, recarrega o candidato pelos loaders geografico e de estados e
compara semantica global, indices, cobertura, diagnosticos estruturais e bytes.
Planos stale, Blocked, caminhos inseguros, colisoes e fontes alteradas falham
antes da copia. ReviewRequired exige acao explicita e resulta no maximo em
`PassedWithReview`. O workspace e removido por padrao em sucesso, falha ou
cancelamento, e a origem e verificada novamente sem ser escrita.

Status: concluida.

## Fase 4C - backup verificavel, Save transacional e rollback

Somente o plano Safe atual cujo digest corresponde ao relatorio 4B atual com
status exatamente `Passed` pode iniciar Save. O fluxo exige confirmacao
explicita, revalida as fontes, adquire lock exclusivo, grava journal duravel,
cria backup fisico com manifesto e verificacao byte a byte, prepara stages ao
lado dos destinos e faz commit deterministico por rename.

Modified e removed preservam o original em rollback path ate um reload real
confirmar semantica, indices, cobertura, victory points, buildings,
diagnosticos, bytes finais e imutabilidade dos arquivos de mapa. Falhas apos o
primeiro rename executam rollback integral em ordem inversa; rollback
incompleto mantem lock, journal, backup e relatorio critico. Um lock encontrado
no proximo carregamento bloqueia edicao ate recovery explicito por rollback.

No sucesso, o projeto recarregado vira o novo baseline, Undo/Redo e dirty sao
zerados e backups permanecem. `Ctrl+S` usa esse fluxo somente quando elegivel.
`Save As`, autosave, restauracao grafica de backups, retencao avancada,
integracao Git e execucao do HOI4 continuam fora do escopo.

Status: concluida.

## Fase 5A - State Inspector compacto e catalogos

O painel tecnico deixou de ser a interface padrao. Um State Inspector lateral
usa um `MapViewport` explicito, busca estados por ID/nome, mostra propriedades
estruturadas e contextualiza diagnosticos sem interceptar ferramentas do mapa.
Os controles reutilizam os drafts e comandos transacionais existentes.

Labels de provincia possuem modos Off, Hovered, Selected State e All.
Diagnosticos de desenvolvedor ficam Off por padrao. Arquivos carregados podem
ser abertos com seguranca; estados criados permanecem sem source. O catalogo em
memoria agrega definitions do mod, base game opcional, fallbacks e valores
customizados observados, sem alterar o pipeline 4A/4B/4C.

Status: em andamento. A base existe, mas o acabamento de UX ainda nao esta
fechado.

## Fase 5A.1 - UX, branding e navegacao de mapa

Corrigir hitboxes compartilhados do Inspector, usar pickers pesquisaveis,
oferecer selecao de tags pelo mapa, expor Select/Pan/Lasso/Brush/Fill na barra
lateral e implementar State Fill sem novo caminho transacional. Atualizar a
marca publica e documentar o workflow seguro mantendo os nomes tecnicos legados
marcados como internos.

Status: em andamento.

## Fase 5A.1.1 - Workspace, Map Views e overlays

Separar o workspace de edicao da apresentacao visual. Expor Province Colors,
Province Types, Terrain/Biome, Continents, Coastal, States e Political como
Map Views distintas. Manter rivers, adjacencies, IDs, fronteiras e uma imagem
de referencia como overlays independentes. Reorganizar o Inspector sem alterar
working state, patching, Save ou persistencia.

Status: em andamento.

## Futuro

- Completar polimento visual e widgets estruturados sem alargar a fronteira de
  escrita.
- Ampliar validacoes de estado e diagnosticos quando houver criterio concreto.
- Consolidar testes no Windows, documentacao de backup, builds reproduziveis e
  publicacao somente apos o fluxo lossless estar comprovado.

## Proxima etapa

Continuar Fase 5A/5A.1. O nucleo das Fases 0 a 4C esta implementado; Save
continua restrito aos arquivos diretos `history/states/*.txt` autorizados pelo
pipeline lossless.
