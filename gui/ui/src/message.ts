import { createDiscreteApi, darkTheme } from "naive-ui";

// 独立于组件树的 message 提示（各 composable / 组件共用）
export const { message } = createDiscreteApi(["message"], {
  configProviderProps: { theme: darkTheme },
});
