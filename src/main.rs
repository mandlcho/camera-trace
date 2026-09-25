use js_sys::{Object, Reflect};
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Event, File, HtmlInputElement, HtmlVideoElement, InputEvent, MediaStream,
    MediaStreamConstraints, PointerEvent,
};
use yew::prelude::*;

#[wasm_bindgen(inline_js = r#"
const DB_NAME = 'trace-projects';
const STORE = 'projects';

function db() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, 1);
    request.onupgradeneeded = () => {
      const database = request.result;
      if (!database.objectStoreNames.contains(STORE)) {
        database.createObjectStore(STORE, { keyPath: 'id' });
      }
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

export async function read_image(file) {
  const url = URL.createObjectURL(file);
  try {
    const image = new Image();
    image.src = url;
    await image.decode();
    const maxSide = 1800;
    const ratio = Math.min(1, maxSide / Math.max(image.naturalWidth, image.naturalHeight));
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(image.naturalWidth * ratio));
    canvas.height = Math.max(1, Math.round(image.naturalHeight * ratio));
    canvas.getContext('2d').drawImage(image, 0, 0, canvas.width, canvas.height);
    return canvas.toDataURL('image/jpeg', 0.88);
  } finally {
    URL.revokeObjectURL(url);
  }
}

export async function save_project(json) {
  const database = await db();
  const project = JSON.parse(json);
  await new Promise((resolve, reject) => {
    const tx = database.transaction(STORE, 'readwrite');
    tx.objectStore(STORE).put(project);
    tx.oncomplete = resolve;
    tx.onerror = () => reject(tx.error);
  });
  database.close();
}

export async function load_projects() {
  const database = await db();
  const projects = await new Promise((resolve, reject) => {
    const request = database.transaction(STORE).objectStore(STORE).getAll();
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  database.close();
  projects.sort((a, b) => b.updated_at - a.updated_at);
  return JSON.stringify(projects.slice(0, 12));
}

export async function delete_project(id) {
  const database = await db();
  await new Promise((resolve, reject) => {
    const tx = database.transaction(STORE, 'readwrite');
    tx.objectStore(STORE).delete(id);
    tx.oncomplete = resolve;
    tx.onerror = () => reject(tx.error);
  });
  database.close();
}
"#)]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn read_image(file: File) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    async fn save_project(json: String) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    async fn load_projects() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    async fn delete_project(id: String) -> Result<JsValue, JsValue>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Project {
    id: String,
    image: String,
    opacity: f64,
    rotation: f64,
    scale: f64,
    x: f64,
    y: f64,
    flipped: bool,
    updated_at: f64,
}

impl Project {
    fn new(image: String) -> Self {
        let now = js_sys::Date::now();
        Self {
            id: format!("project-{now}"),
            image,
            opacity: 0.5,
            rotation: 0.0,
            scale: 1.0,
            x: 0.0,
            y: 0.0,
            flipped: false,
            updated_at: now,
        }
    }
}

fn js_error(value: JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| "The browser rejected the request.".into())
}

#[function_component(App)]
fn app() -> Html {
    let video_ref = use_node_ref();
    let file_input_ref = use_node_ref();
    let current = use_state(|| None::<Project>);
    let history = use_state(Vec::<Project>::new);
    let camera_on = use_state(|| false);
    let camera_error = use_state(|| None::<String>);
    let busy = use_state(|| false);
    let controls_open = use_state(|| true);
    let drag_origin = use_mut_ref(|| None::<(f64, f64, f64, f64)>);

    {
        let history = history.clone();
        use_effect_with((), move |_| {
            spawn_local(async move {
                if let Ok(value) = load_projects().await
                    && let Some(json) = value.as_string()
                    && let Ok(projects) = serde_json::from_str::<Vec<Project>>(&json)
                {
                    history.set(projects);
                }
            });
            || ()
        });
    }

    let start_camera = {
        let video_ref = video_ref.clone();
        let camera_on = camera_on.clone();
        let camera_error = camera_error.clone();
        Callback::from(move |_| {
            let video_ref = video_ref.clone();
            let camera_on = camera_on.clone();
            let camera_error = camera_error.clone();
            spawn_local(async move {
                camera_error.set(None);
                let result = async {
                    let window = web_sys::window().ok_or("No browser window is available.")?;
                    let devices = window.navigator().media_devices().map_err(js_error)?;
                    let video_options = Object::new();
                    Reflect::set(
                        &video_options,
                        &JsValue::from_str("facingMode"),
                        &JsValue::from_str("environment"),
                    )
                    .map_err(js_error)?;
                    let constraints = MediaStreamConstraints::new();
                    constraints.set_audio(&JsValue::FALSE);
                    constraints.set_video(&video_options);
                    let promise = devices
                        .get_user_media_with_constraints(&constraints)
                        .map_err(js_error)?;
                    let stream = JsFuture::from(promise)
                        .await
                        .map_err(js_error)?
                        .dyn_into::<MediaStream>()
                        .map_err(js_error)?;
                    let video = video_ref
                        .cast::<HtmlVideoElement>()
                        .ok_or("The camera view is not ready.")?;
                    video.set_src_object(Some(&stream));
                    let _ = video.play().map(JsFuture::from).map_err(js_error)?.await;
                    Ok::<(), String>(())
                }
                .await;

                match result {
                    Ok(()) => camera_on.set(true),
                    Err(message) => camera_error.set(Some(message)),
                }
            });
        })
    };

    let choose_photo = {
        let file_input_ref = file_input_ref.clone();
        Callback::from(move |_| {
            if let Some(input) = file_input_ref.cast::<HtmlInputElement>() {
                input.click();
            }
        })
    };

    let on_file = {
        let current = current.clone();
        let history = history.clone();
        let busy = busy.clone();
        Callback::from(move |event: Event| {
            let input: HtmlInputElement = event.target_unchecked_into();
            let Some(file) = input.files().and_then(|files| files.item(0)) else {
                return;
            };
            let current = current.clone();
            let history = history.clone();
            let busy = busy.clone();
            busy.set(true);
            spawn_local(async move {
                if let Ok(value) = read_image(file).await
                    && let Some(image) = value.as_string()
                {
                    let project = Project::new(image);
                    if let Ok(json) = serde_json::to_string(&project) {
                        let _ = save_project(json).await;
                    }
                    let mut next = (*history).clone();
                    next.insert(0, project.clone());
                    next.truncate(12);
                    history.set(next);
                    current.set(Some(project));
                }
                busy.set(false);
            });
        })
    };

    let update_project = {
        let current = current.clone();
        let history = history.clone();
        Callback::from(move |mut project: Project| {
            project.updated_at = js_sys::Date::now();
            current.set(Some(project.clone()));
            let mut next = (*history).clone();
            if let Some(existing) = next.iter_mut().find(|item| item.id == project.id) {
                *existing = project.clone();
            } else {
                next.insert(0, project.clone());
            }
            next.sort_by(|a, b| b.updated_at.total_cmp(&a.updated_at));
            next.truncate(12);
            history.set(next);
            spawn_local(async move {
                if let Ok(json) = serde_json::to_string(&project) {
                    let _ = save_project(json).await;
                }
            });
        })
    };

    let on_pointer_down = {
        let current = current.clone();
        let drag_origin = drag_origin.clone();
        Callback::from(move |event: PointerEvent| {
            if let Some(project) = current.as_ref() {
                event.prevent_default();
                if let Some(target) = event.current_target() {
                    let _ = target
                        .dyn_into::<web_sys::Element>()
                        .map(|element| element.set_pointer_capture(event.pointer_id()));
                }
                *drag_origin.borrow_mut() = Some((
                    event.client_x() as f64,
                    event.client_y() as f64,
                    project.x,
                    project.y,
                ));
            }
        })
    };

    let on_pointer_move = {
        let current = current.clone();
        let drag_origin = drag_origin.clone();
        Callback::from(move |event: PointerEvent| {
            let Some((start_x, start_y, image_x, image_y)) = *drag_origin.borrow() else {
                return;
            };
            if let Some(mut project) = (*current).clone() {
                project.x = image_x + event.client_x() as f64 - start_x;
                project.y = image_y + event.client_y() as f64 - start_y;
                current.set(Some(project));
            }
        })
    };

    let on_pointer_up = {
        let current = current.clone();
        let drag_origin = drag_origin.clone();
        let update_project = update_project.clone();
        Callback::from(move |_: PointerEvent| {
            *drag_origin.borrow_mut() = None;
            if let Some(project) = (*current).clone() {
                update_project.emit(project);
            }
        })
    };

    let set_number = |field: &'static str| {
        let current = current.clone();
        let update_project = update_project.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlInputElement = event.target_unchecked_into();
            if let Some(mut project) = (*current).clone() {
                let value = input.value_as_number();
                match field {
                    "opacity" => project.opacity = value,
                    "rotation" => project.rotation = value,
                    "scale" => project.scale = value,
                    _ => {}
                }
                update_project.emit(project);
            }
        })
    };

    let flip = {
        let current = current.clone();
        let update_project = update_project.clone();
        Callback::from(move |_| {
            if let Some(mut project) = (*current).clone() {
                project.flipped = !project.flipped;
                update_project.emit(project);
            }
        })
    };

    let reset = {
        let current = current.clone();
        let update_project = update_project.clone();
        Callback::from(move |_| {
            if let Some(mut project) = (*current).clone() {
                project.opacity = 0.5;
                project.rotation = 0.0;
                project.scale = 1.0;
                project.x = 0.0;
                project.y = 0.0;
                project.flipped = false;
                update_project.emit(project);
            }
        })
    };

    let remove_current = {
        let current = current.clone();
        let history = history.clone();
        Callback::from(move |_| {
            let Some(project) = (*current).clone() else {
                return;
            };
            let mut next = (*history).clone();
            next.retain(|item| item.id != project.id);
            current.set(next.first().cloned());
            history.set(next);
            spawn_local(async move {
                let _ = delete_project(project.id).await;
            });
        })
    };

    let overlay = current.as_ref().map(|project| {
        let flip_x = if project.flipped { -1.0 } else { 1.0 };
        let style = format!(
            "opacity:{};transform:translate(calc(-50% + {}px),calc(-50% + {}px)) rotate({}deg) scale({},{})",
            project.opacity, project.x, project.y, project.rotation, project.scale * flip_x, project.scale
        );
        html! { <img class="reference" src={project.image.clone()} alt="Reference overlay" {style} draggable="false" /> }
    });

    html! {
        <main class="app-shell">
            <header class="topbar">
                <div>
                    <span class="eyebrow">{"TRACING CAMERA"}</span>
                    <h1>{"Trace"}</h1>
                </div>
                <button class="icon-button" onclick={{
                    let controls_open = controls_open.clone();
                    Callback::from(move |_| controls_open.set(!*controls_open))
                }} aria-label="Toggle controls">{if *controls_open { "Hide" } else { "Edit" }}</button>
            </header>

            <section
                class="viewport"
                onpointerdown={on_pointer_down}
                onpointermove={on_pointer_move}
                onpointerup={on_pointer_up.clone()}
                onpointercancel={on_pointer_up}
            >
                <video ref={video_ref} autoplay=true playsinline=true muted=true></video>
                {overlay.unwrap_or_else(|| html! {
                    <div class="empty-state">
                        <div class="empty-mark">{"＋"}</div>
                        <p>{"Add a reference photo to begin tracing."}</p>
                    </div>
                })}
                if !*camera_on {
                    <div class="camera-prompt">
                        <button class="primary" onclick={start_camera}>{"Start camera"}</button>
                        if let Some(message) = camera_error.as_ref() {
                            <p class="error">{message}</p>
                        }
                    </div>
                }
                <div class="view-hint">{"Drag the image to position it"}</div>
            </section>

            if *controls_open {
                <section class="control-panel">
                    <input
                        ref={file_input_ref}
                        class="visually-hidden"
                        type="file"
                        accept="image/*"
                        onchange={on_file}
                    />
                    <div class="action-row">
                        <button class="primary" onclick={choose_photo} disabled={*busy}>
                            {if *busy { "Preparing…" } else if current.is_some() { "Replace photo" } else { "Choose photo" }}
                        </button>
                        <button onclick={flip} disabled={current.is_none()}>{"Flip"}</button>
                        <button onclick={reset} disabled={current.is_none()}>{"Reset"}</button>
                    </div>

                    if let Some(project) = current.as_ref() {
                        <div class="sliders">
                            <label>
                                <span>{"Opacity"}<output>{format!("{}%", (project.opacity * 100.0).round())}</output></span>
                                <input type="range" min="0.05" max="1" step="0.01" value={project.opacity.to_string()} oninput={set_number("opacity")} />
                            </label>
                            <label>
                                <span>{"Scale"}<output>{format!("{:.1}×", project.scale)}</output></span>
                                <input type="range" min="0.2" max="3" step="0.05" value={project.scale.to_string()} oninput={set_number("scale")} />
                            </label>
                            <label>
                                <span>{"Rotation"}<output>{format!("{}°", project.rotation.round())}</output></span>
                                <input type="range" min="-180" max="180" step="1" value={project.rotation.to_string()} oninput={set_number("rotation")} />
                            </label>
                        </div>
                    }
                </section>
            }

            if !history.is_empty() {
                <section class="history">
                    <div class="section-heading">
                        <h2>{"Recent"}</h2>
                        <button class="danger-link" onclick={remove_current} disabled={current.is_none()}>{"Delete current"}</button>
                    </div>
                    <div class="history-strip">
                        {for history.iter().map(|project| {
                            let selected = current.as_ref().is_some_and(|item| item.id == project.id);
                            let item = project.clone();
                            let current = current.clone();
                            html! {
                                <button class={classes!("history-item", selected.then_some("selected"))} onclick={Callback::from(move |_| current.set(Some(item.clone())))}>
                                    <img src={project.image.clone()} alt="Saved reference" />
                                </button>
                            }
                        })}
                    </div>
                </section>
            }
        </main>
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}
