/**
 * 轨道栏与编排网格共享的纵向滚动位置（像素）。
 * 两侧各自监听自己的滚动容器，把位置写进这里；再 watch 它回写对侧，
 * 从而让音轨行始终与编排的 lane 对齐。
 */
import { ref } from "vue";

/** 当前纵向滚动位置。 */
export const laneScrollTop = ref(0);
