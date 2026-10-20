/** 取路径最后一段作为显示名 */
export const shortName = (p: string) => p.split(/[\\/]/).pop() ?? p;
