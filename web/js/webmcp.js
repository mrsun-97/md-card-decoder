// WebMCP tools (when the browser exposes a model context): let an assistant read the state, ask for
// a recommendation and record what the game showed.
export function registerWebMcp(api) {
  const context = navigator.modelContext || document.modelContext;
  if (!context?.registerTool) return;
  const register = (tool) => Promise.resolve(context.registerTool(tool)).catch(() => {});
  const fields = ['frame', 'attribute', 'race', 'level', 'atk', 'def'];
  register({
    name: 'read_solver_state',
    title: '读取求解器状态',
    description: '读取当前题号、可用与计划的提示/挑战次数、已知字段和候选数量，不修改页面状态。',
    inputSchema: { type: 'object', properties: {}, additionalProperties: false },
    annotations: { readOnlyHint: true },
    execute: () => api.summary(),
  });
  register({
    name: 'get_recommendation',
    title: '获取推荐行动',
    description: '计算下一步应先使用提示还是挑战哪张卡，并给出求解方法与预期解题数。',
    inputSchema: { type: 'object', properties: {}, additionalProperties: false },
    annotations: { readOnlyHint: true },
    execute: () => api.recommendation(),
  });
  register({
    name: 'record_reveal',
    title: '录入揭示字段',
    description: '录入游戏中的初始揭示或提示结果。value 为原始数值：边框为位掩码，属性/种族为位值，等级为数字，攻守为数字（? 为 -2）。',
    inputSchema: {
      type: 'object',
      properties: { field: { type: 'string', enum: fields }, value: { type: 'number' }, source: { type: 'string', enum: ['initial', 'hint'] } },
      required: ['field', 'value', 'source'],
      additionalProperties: false,
    },
    execute: (i) => api.reveal(i.field, i.value, i.source),
  });
  register({
    name: 'record_challenge',
    title: '录入挑战判定',
    description: '录入一次挑战：卡片密码、游戏中显示为相符的字段，以及边框/等级相符时揭示的值。',
    inputSchema: {
      type: 'object',
      properties: {
        passcode: { type: 'number' },
        matched: { type: 'array', items: { type: 'string', enum: fields } },
        frame: { type: 'number' },
        level: { type: 'number' },
      },
      required: ['passcode', 'matched'],
      additionalProperties: false,
    },
    execute: (i) => api.challenge(i.passcode, i.matched, i.frame, i.level),
  });
}
