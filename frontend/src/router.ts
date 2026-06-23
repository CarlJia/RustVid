import { createRouter, createWebHashHistory } from "vue-router";
import HomeView from "./views/HomeView.vue";
import JobView from "./views/JobView.vue";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", component: HomeView, name: "home" },
    { path: "/jobs/:id", component: JobView, name: "job", props: true },
  ],
});
