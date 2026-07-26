# Plano de migração

## Fase 0 — preparar e estabilizar a base

Identificar o fork, documentar a arquitetura herdada, validar a raiz do mod,
modelar estados sem parser e tornar o mapa somente leitura em projetos de
estado.

## Fase 1A — carregar projeto e indexar estados (concluída)

Enumeração determinística, lexer e parser genéricos, árvore com spans e trivia,
extração tipada, `states_by_id`, `state_by_province`,
`ambiguous_provinces`, diagnósticos e resumo de carregamento foram
implementados. Arquivos inválidos não interrompem os demais.

## Fase 1B — visualização e seleção de estados (concluída)

Resolver `pixel -> RGB -> province ID -> state ID` e gerar uma textura de
estados sem alterar o bitmap geográfico. A visualização usa cores
determinísticas por ID, bordas entre estados e cores reservadas para
províncias ambíguas, terrestres sem estado e referências provinciais
inexistentes. O clique seleciona o estado inteiro e mostra informações
básicas somente para leitura.

## Fase 2A — edição transacional em memória (concluída)

Reutilizar picking para mudar somente associações entre províncias terrestres
e estados existentes em uma sessão temporária. A implementação atual mantém
baseline e working set separados, histórico próprio de undo/redo, descarte
explícito, prompt de descarte ao fechar/trocar projeto, bloqueio de `Ctrl+S`,
seleção por Ctrl+click, estado alvo por clique normal, Move, Unassign e
seleção em lote das províncias do alvo e atualização da
textura/contadores/diagnósticos a partir do working set.

Não há serializer, diff, backup, salvamento, criação/exclusão de estados,
merge/split, brush ou edição de propriedades nesta fase.

## Fase 2B — seleção avançada e UX de edição (implementada)

O lasso específico de estados captura um polígono em coordenadas do mapa,
calcula uma prévia por bounding box e seleciona províncias terrestres inteiras.
Replace, Add e Remove controlam a combinação com a seleção atual.
CentroidInside é o padrão; AnyIntersection e MajorityInside ficam disponíveis
no menu da ferramenta. Mar, lago, IDs desconhecidos, ambiguidades e estados
inválidos não entram silenciosamente na seleção editável.

Confirmar a prévia altera somente `selected_provinces`. Move e Unassign
reutilizam o comando transacional da Fase 2A, portanto o lote é atômico e gera
uma única entrada de undo. Regiões pequenas recebem atualização visual
seletiva; regiões grandes usam rebuild completo. Nada é serializado.

## Fase 3A — propriedades de estado em memória (concluída)

Nome lógico, manpower, category, owner/controller, cores, claims, recursos e
construções estaduais são editados por draft validado e um comando atômico.

## Fase 3B — dados provinciais em memória (concluída)

Victory points e construções provinciais são editados no estado ativo e
acompanham a província em Move, Unassign, Undo, Redo e Discard.

## Fase 3C — criação e remoção controlada em memória (concluída)

Criar estado vazio ou a partir da seleção e remover estado com Move All ou
Unassign All usam o mesmo working set e histórico transacional. IDs ocupados
ou reservados são rejeitados, estados carregados removidos viram tombstones da
sessão e snapshots completos tornam Undo/Redo reversíveis. Nenhum arquivo de
estado é criado, apagado, renomeado ou escrito.

## Fase 3D — State Brush em memória

Atribuir províncias diretamente ao estado alvo com um brush que reutilize os
mesmos preflights, deltas provinciais e histórico transacional, ainda sem
serializer ou escrita em disco.

## Fase 4 — salvamento lossless

Usar os tokens e spans já preservados para editar somente intervalos afetados,
mantendo comentários, formatação, ordem, chaves repetidas, blocos datados e
campos desconhecidos. Salvar somente arquivos de estado modificados.

## Fase 5 — validações e diagnósticos avançados

Detectar IDs duplicados, províncias em vários estados, províncias terrestres
sem estado, categorias ausentes e referências provinciais incoerentes.

## Fase 6 — empacotamento e releases

Consolidar testes no Windows, documentação de backup, builds reproduzíveis e
publicação somente após o fluxo lossless estar comprovado.

## Próxima etapa

Fase 3D: implementar State Brush para atribuição direta de províncias sobre o
modelo transacional em memória. Depois dela, a Fase 4A preparará patches
lossless sobre os tokens e spans preservados, ainda sem gravar automaticamente
nos arquivos reais. O mapa geográfico e os arquivos de estado permanecem sem
salvamento até esse mecanismo ser validado.
