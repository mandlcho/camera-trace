use js_sys::{Object, Reflect};
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Event, File, HtmlElement, HtmlInputElement, HtmlVideoElement, InputEvent, KeyboardEvent,
    MediaStream, MediaStreamConstraints, MouseEvent, PointerEvent, TouchEvent,
};
use yew::prelude::*;

const ONBOARDING_KEY: &str = "camera-trace:onboarding:v1";
const ALIGNMENT_HINT_KEY: &str = "camera-trace:alignment-hint:v1";
const DISMISSED_VALUE: &str = "dismissed";

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
    async fn save_project(json: String) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    async fn load_projects() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    async fn delete_project(id: String) -> Result<JsValue, JsValue>;
}

#[wasm_bindgen(module = "/image-import.js")]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn read_image(file: File) -> Result<JsValue, JsValue>;
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

    fn set_number(&mut self, field: &str, value: f64, position_locked: bool) -> bool {
        match field {
            "opacity" => self.opacity = value,
            "rotation" if !position_locked => self.rotation = value,
            "scale" if !position_locked => self.scale = value,
            _ => return false,
        }
        true
    }

    fn move_to(&mut self, x: f64, y: f64, position_locked: bool) -> bool {
        if position_locked {
            return false;
        }
        self.x = x;
        self.y = y;
        true
    }

    fn flip(&mut self, position_locked: bool) -> bool {
        if position_locked {
            return false;
        }
        self.flipped = !self.flipped;
        true
    }

    fn reset(&mut self, position_locked: bool) -> bool {
        if position_locked {
            return false;
        }
        self.opacity = 0.5;
        self.rotation = 0.0;
        self.scale = 1.0;
        self.x = 0.0;
        self.y = 0.0;
        self.flipped = false;
        true
    }
}

fn js_error(value: JsValue) -> String {
    if let Some(message) = value.as_string() {
        return message;
    }
    let name = Reflect::get(&value, &JsValue::from_str("name"))
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default();
    let message = Reflect::get(&value, &JsValue::from_str("message"))
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default();

    match name.as_str() {
        "NotAllowedError" => "Camera access was denied. In Safari, open Page Settings, set Camera to Allow, then reload.".into(),
        "NotFoundError" => "No camera was found on this device.".into(),
        "NotReadableError" => "The camera is already in use by another app or browser tab.".into(),
        "OverconstrainedError" => "The requested camera is unavailable on this device.".into(),
        _ if !message.is_empty() => message,
        _ => "The browser could not start the camera.".into(),
    }
}

fn camera_error_name(value: &JsValue) -> String {
    Reflect::get(value, &JsValue::from_str("name"))
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default()
}

fn stored_value_is_dismissed(value: Option<&str>) -> bool {
    value == Some(DISMISSED_VALUE)
}

fn read_dismissed_preference(key: &str) -> bool {
    web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(key).ok().flatten())
        .is_some_and(|value| stored_value_is_dismissed(Some(&value)))
}

fn persist_dismissed_preference(key: &str) {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let _ = storage.set_item(key, DISMISSED_VALUE);
    }
}

fn should_show_intro(
    history_loaded: bool,
    onboarding_dismissed: bool,
    has_current_project: bool,
    has_saved_projects: bool,
) -> bool {
    history_loaded && !onboarding_dismissed && !has_current_project && !has_saved_projects
}

fn should_dismiss_onboarding(import_succeeded: bool, explicit_skip: bool) -> bool {
    import_succeeded || explicit_skip
}

#[function_component(SetupDiagram)]
fn setup_diagram() -> Html {
    html! {
        <svg class="setup-diagram" viewBox="0 0 320 176" role="img" aria-labelledby="setup-diagram-title setup-diagram-description">
            <title id="setup-diagram-title">{"Phone supported above paper for tracing"}</title>
            <desc id="setup-diagram-description">{"A stable overhead stand holds a phone horizontally with its rear camera facing down and screen facing up. Paper and clear space for a drawing hand are underneath."}</desc>
            <rect class="diagram-paper" x="80" y="112" width="174" height="50" rx="4" />
            <path class="diagram-drawing" d="M112 146c22-27 53-27 78-4 11 10 24 7 34-10" />
            <path class="diagram-stand" d="M30 154V32h52v12H47v110" />
            <path class="diagram-arm" d="M47 51h58" />
            <rect class="diagram-phone" x="92" y="35" width="137" height="66" rx="10" />
            <rect class="diagram-screen" x="100" y="42" width="121" height="52" rx="6" />
            <path class="diagram-image" d="M116 80l21-20 17 14 14-11 34 28h-86z" />
            <circle class="diagram-camera" cx="214" cy="96" r="3" />
            <path class="diagram-direction" d="M207 106v22m-6-6 6 6 6-6" />
            <path class="diagram-hand" d="M270 157c-8-8-10-17-5-25 2-4 5-3 7 1l4 8v-24c0-5 7-5 7 0v16-21c0-5 7-5 7 0v21-17c0-5 7-5 7 0v20-12c0-5 7-5 7 0v19c0 8-4 14-10 18" />
            <text x="108" y="24">{"screen up"}</text>
            <text x="173" y="142">{"paper"}</text>
            <text x="243" y="102">{"hand space"}</text>
            <text x="18" y="170">{"stable overhead stand"}</text>
        </svg>
    }
}

#[function_component(SetupSteps)]
fn setup_steps() -> Html {
    html! {
        <>
            <SetupDiagram />
            <ol class="setup-steps">
                <li>
                    <span>{"1"}</span>
                    <div><strong>{"Support your phone"}</strong><p>{"Use a stable overhead stand, rear camera pointing at the paper. Leave room for your drawing hand."}</p></div>
                </li>
                <li>
                    <span>{"2"}</span>
                    <div><strong>{"Choose and align a photo"}</strong><p>{"Start the camera, then adjust the image’s size, position and opacity."}</p></div>
                </li>
                <li>
                    <span>{"3"}</span>
                    <div><strong>{"Lock and trace"}</strong><p>{"Lock the image position and draw while looking at the screen. Keep the phone still."}</p></div>
                </li>
            </ol>
        </>
    }
}

#[derive(Properties, PartialEq)]
struct GuideSheetProps {
    on_close: Callback<()>,
    close_ref: NodeRef,
}

#[function_component(GuideSheet)]
fn guide_sheet(props: &GuideSheetProps) -> Html {
    let on_keydown = {
        let on_close = props.on_close.clone();
        let close_ref = props.close_ref.clone();
        Callback::from(move |event: KeyboardEvent| match event.key().as_str() {
            "Escape" => {
                event.prevent_default();
                event.stop_propagation();
                on_close.emit(());
            }
            "Tab" => {
                event.prevent_default();
                if let Some(button) = close_ref.cast::<HtmlElement>() {
                    let _ = button.focus();
                }
            }
            _ => {}
        })
    };
    let close_from_backdrop = {
        let on_close = props.on_close.clone();
        Callback::from(move |_| on_close.emit(()))
    };

    html! {
        <div class="guide-backdrop" onclick={close_from_backdrop} onkeydown={on_keydown}>
            <section
                class="guide-sheet"
                role="dialog"
                aria-modal="true"
                aria-labelledby="guide-title"
                onclick={Callback::from(|event: MouseEvent| event.stop_propagation())}
            >
                <header class="guide-header">
                    <div>
                        <p class="eyebrow">{"Camera Trace"}</p>
                        <h2 id="guide-title">{"Setup guide"}</h2>
                    </div>
                    <button ref={props.close_ref.clone()} class="icon-button" aria-label="Close setup guide" onclick={{
                        let on_close = props.on_close.clone();
                        Callback::from(move |_| on_close.emit(()))
                    }}>{"×"}</button>
                </header>
                <p class="guide-clarification">{"Your reference is a screen overlay—it isn’t physically projected onto the paper."}</p>
                <SetupSteps />
                <p class="privacy-note">{"Free to use. No account needed. Photos are processed and saved in this browser."}</p>
            </section>
        </div>
    }
}

#[function_component(App)]
fn app() -> Html {
    let video_ref = use_node_ref();
    let file_input_ref = use_node_ref();
    let setup_guide_trigger_ref = use_node_ref();
    let guide_close_ref = use_node_ref();
    let camera_stream = use_mut_ref(|| None::<MediaStream>);
    let current = use_state(|| None::<Project>);
    let history = use_state(Vec::<Project>::new);
    let history_loaded = use_state(|| false);
    let camera_on = use_state(|| false);
    let camera_error = use_state(|| None::<String>);
    let import_error = use_state(|| None::<String>);
    let busy = use_state(|| false);
    let overlay_hidden = use_state(|| false);
    let position_locked = use_state(|| false);
    let onboarding_dismissed = use_state(|| read_dismissed_preference(ONBOARDING_KEY));
    let alignment_hint_dismissed = use_state(|| read_dismissed_preference(ALIGNMENT_HINT_KEY));
    let guide_open = use_state(|| false);
    // A mutable mirror makes the lock effective synchronously, including for callbacks
    // from a gesture or control that was attached before the locking render.
    let position_locked_ref = use_mut_ref(|| false);
    // start x/y, original image x/y, and the latest project state
    let drag_state = use_mut_ref(|| None::<(f64, f64, f64, f64, Project)>);

    {
        let history = history.clone();
        let history_loaded = history_loaded.clone();
        let onboarding_dismissed = onboarding_dismissed.clone();
        use_effect_with((), move |_| {
            spawn_local(async move {
                if let Ok(value) = load_projects().await
                    && let Some(json) = value.as_string()
                    && let Ok(projects) = serde_json::from_str::<Vec<Project>>(&json)
                {
                    if !projects.is_empty() {
                        onboarding_dismissed.set(true);
                        persist_dismissed_preference(ONBOARDING_KEY);
                    }
                    history.set(projects);
                }
                history_loaded.set(true);
            });
            || ()
        });
    }

    {
        let guide_open = guide_open.clone();
        let guide_close_ref = guide_close_ref.clone();
        use_effect_with(*guide_open, move |is_open| {
            if *is_open && let Some(close_button) = guide_close_ref.cast::<HtmlElement>() {
                let _ = close_button.focus();
            }
            || ()
        });
    }

    let start_camera = {
        let video_ref = video_ref.clone();
        let camera_on = camera_on.clone();
        let camera_error = camera_error.clone();
        let camera_stream = camera_stream.clone();
        Callback::from(move |_| {
            let video_ref = video_ref.clone();
            let camera_on = camera_on.clone();
            let camera_error = camera_error.clone();
            let camera_stream = camera_stream.clone();
            spawn_local(async move {
                camera_error.set(None);
                let result = async {
                    let window = web_sys::window().ok_or("No browser window is available.")?;
                    let devices = window.navigator().media_devices().map_err(js_error)?;
                    let video = video_ref
                        .cast::<HtmlVideoElement>()
                        .ok_or("The camera view is not ready.")?;

                    // Set DOM properties directly. The HTML `muted` attribute alone does
                    // not reliably update the runtime property in iOS Safari.
                    video.set_muted(true);
                    video.set_autoplay(true);
                    Reflect::set(
                        video.as_ref(),
                        &JsValue::from_str("playsInline"),
                        &JsValue::TRUE,
                    )
                    .map_err(js_error)?;

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
                    let stream_value = match JsFuture::from(promise).await {
                        Ok(stream) => stream,
                        Err(error)
                            if matches!(
                                camera_error_name(&error).as_str(),
                                "NotFoundError" | "OverconstrainedError"
                            ) =>
                        {
                            let fallback = MediaStreamConstraints::new();
                            fallback.set_audio(&JsValue::FALSE);
                            fallback.set_video(&JsValue::TRUE);
                            let promise = devices
                                .get_user_media_with_constraints(&fallback)
                                .map_err(js_error)?;
                            JsFuture::from(promise).await.map_err(js_error)?
                        }
                        Err(error) => return Err(js_error(error)),
                    };
                    let stream = stream_value.dyn_into::<MediaStream>().map_err(js_error)?;
                    video.set_src_object(Some(&stream));
                    *camera_stream.borrow_mut() = Some(stream);
                    JsFuture::from(video.play().map_err(js_error)?)
                        .await
                        .map_err(js_error)?;
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
        let import_error = import_error.clone();
        Callback::from(move |_| {
            import_error.set(None);
            if let Some(input) = file_input_ref.cast::<HtmlInputElement>() {
                input.click();
            }
        })
    };

    let on_file = {
        let current = current.clone();
        let history = history.clone();
        let busy = busy.clone();
        let import_error = import_error.clone();
        let onboarding_dismissed = onboarding_dismissed.clone();
        let overlay_hidden = overlay_hidden.clone();
        let position_locked = position_locked.clone();
        let position_locked_ref = position_locked_ref.clone();
        let drag_state = drag_state.clone();
        Callback::from(move |event: Event| {
            let input: HtmlInputElement = event.target_unchecked_into();
            let Some(file) = input.files().and_then(|files| files.item(0)) else {
                return;
            };
            input.set_value("");
            let current = current.clone();
            let history = history.clone();
            let busy = busy.clone();
            let import_error = import_error.clone();
            let onboarding_dismissed = onboarding_dismissed.clone();
            let overlay_hidden = overlay_hidden.clone();
            let position_locked = position_locked.clone();
            let position_locked_ref = position_locked_ref.clone();
            let drag_state = drag_state.clone();
            busy.set(true);
            spawn_local(async move {
                match read_image(file).await {
                    Ok(value) => {
                        if let Some(image) = value.as_string() {
                            let project = Project::new(image);
                            if let Ok(json) = serde_json::to_string(&project) {
                                let _ = save_project(json).await;
                            }
                            let mut next = (*history).clone();
                            next.insert(0, project.clone());
                            next.truncate(12);
                            history.set(next);
                            current.set(Some(project));
                            overlay_hidden.set(false);
                            position_locked.set(false);
                            *position_locked_ref.borrow_mut() = false;
                            *drag_state.borrow_mut() = None;
                            import_error.set(None);
                            if should_dismiss_onboarding(true, false) {
                                onboarding_dismissed.set(true);
                                persist_dismissed_preference(ONBOARDING_KEY);
                            }
                        } else {
                            import_error.set(Some(
                                "This image could not be prepared. Try another photo.".into(),
                            ));
                        }
                    }
                    Err(_) => import_error.set(Some(
                        "This image could not be prepared. Try another photo.".into(),
                    )),
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
        let drag_state = drag_state.clone();
        let position_locked_ref = position_locked_ref.clone();
        Callback::from(move |event: PointerEvent| {
            if *position_locked_ref.borrow() {
                return;
            }
            if let Some(project) = current.as_ref() {
                event.prevent_default();
                if let Some(target) = event.current_target() {
                    let _ = target
                        .dyn_into::<web_sys::Element>()
                        .map(|element| element.set_pointer_capture(event.pointer_id()));
                }
                *drag_state.borrow_mut() = Some((
                    event.client_x() as f64,
                    event.client_y() as f64,
                    project.x,
                    project.y,
                    project.clone(),
                ));
            }
        })
    };

    let on_pointer_move = {
        let current = current.clone();
        let drag_state = drag_state.clone();
        let position_locked_ref = position_locked_ref.clone();
        Callback::from(move |event: PointerEvent| {
            if *position_locked_ref.borrow() {
                *drag_state.borrow_mut() = None;
                return;
            }
            let mut drag = drag_state.borrow_mut();
            let Some((start_x, start_y, image_x, image_y, project)) = drag.as_mut() else {
                return;
            };
            event.prevent_default();
            let x = *image_x + event.client_x() as f64 - *start_x;
            let y = *image_y + event.client_y() as f64 - *start_y;
            if project.move_to(x, y, *position_locked_ref.borrow()) {
                current.set(Some(project.clone()));
            }
        })
    };

    let on_pointer_up = {
        let drag_state = drag_state.clone();
        let update_project = update_project.clone();
        let position_locked_ref = position_locked_ref.clone();
        Callback::from(move |_: PointerEvent| {
            if *position_locked_ref.borrow() {
                *drag_state.borrow_mut() = None;
                return;
            }
            if let Some((_, _, _, _, project)) = drag_state.borrow_mut().take() {
                update_project.emit(project);
            }
        })
    };

    // Explicit touch handlers are kept alongside pointer events because older
    // iOS WebKit versions can lose pointer capture when the camera video is active.
    let on_touch_start = {
        let current = current.clone();
        let drag_state = drag_state.clone();
        let position_locked_ref = position_locked_ref.clone();
        Callback::from(move |event: TouchEvent| {
            if *position_locked_ref.borrow() {
                return;
            }
            let Some(touch) = event.touches().item(0) else {
                return;
            };
            if let Some(project) = current.as_ref() {
                event.prevent_default();
                *drag_state.borrow_mut() = Some((
                    touch.client_x() as f64,
                    touch.client_y() as f64,
                    project.x,
                    project.y,
                    project.clone(),
                ));
            }
        })
    };

    let on_touch_move = {
        let current = current.clone();
        let drag_state = drag_state.clone();
        let position_locked_ref = position_locked_ref.clone();
        Callback::from(move |event: TouchEvent| {
            if *position_locked_ref.borrow() {
                *drag_state.borrow_mut() = None;
                return;
            }
            let Some(touch) = event.touches().item(0) else {
                return;
            };
            let mut drag = drag_state.borrow_mut();
            let Some((start_x, start_y, image_x, image_y, project)) = drag.as_mut() else {
                return;
            };
            event.prevent_default();
            let x = *image_x + touch.client_x() as f64 - *start_x;
            let y = *image_y + touch.client_y() as f64 - *start_y;
            if project.move_to(x, y, *position_locked_ref.borrow()) {
                current.set(Some(project.clone()));
            }
        })
    };

    let on_touch_end = {
        let drag_state = drag_state.clone();
        let update_project = update_project.clone();
        let position_locked_ref = position_locked_ref.clone();
        Callback::from(move |event: TouchEvent| {
            event.prevent_default();
            if *position_locked_ref.borrow() {
                *drag_state.borrow_mut() = None;
                return;
            }
            if let Some((_, _, _, _, project)) = drag_state.borrow_mut().take() {
                update_project.emit(project);
            }
        })
    };

    let set_number = |field: &'static str| {
        let current = current.clone();
        let update_project = update_project.clone();
        let position_locked_ref = position_locked_ref.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlInputElement = event.target_unchecked_into();
            if let Some(mut project) = (*current).clone() {
                let value = input.value_as_number();
                if project.set_number(field, value, *position_locked_ref.borrow()) {
                    update_project.emit(project);
                }
            }
        })
    };

    let flip = {
        let current = current.clone();
        let update_project = update_project.clone();
        let position_locked_ref = position_locked_ref.clone();
        Callback::from(move |_| {
            if let Some(mut project) = (*current).clone()
                && project.flip(*position_locked_ref.borrow())
            {
                update_project.emit(project);
            }
        })
    };

    let toggle_overlay = {
        let overlay_hidden = overlay_hidden.clone();
        Callback::from(move |_| overlay_hidden.set(!*overlay_hidden))
    };

    let reset = {
        let current = current.clone();
        let update_project = update_project.clone();
        let position_locked_ref = position_locked_ref.clone();
        Callback::from(move |_| {
            if let Some(mut project) = (*current).clone()
                && project.reset(*position_locked_ref.borrow())
            {
                update_project.emit(project);
            }
        })
    };

    let toggle_position_lock = {
        let current = current.clone();
        let position_locked = position_locked.clone();
        let position_locked_ref = position_locked_ref.clone();
        let drag_state = drag_state.clone();
        let update_project = update_project.clone();
        let alignment_hint_dismissed = alignment_hint_dismissed.clone();
        Callback::from(move |_| {
            if current.is_some() {
                let next = !*position_locked_ref.borrow();
                *position_locked_ref.borrow_mut() = next;
                let active_drag = drag_state.borrow_mut().take();
                if next && let Some((_, _, _, _, project)) = active_drag {
                    update_project.emit(project);
                }
                position_locked.set(next);
                if next {
                    alignment_hint_dismissed.set(true);
                    persist_dismissed_preference(ALIGNMENT_HINT_KEY);
                }
            }
        })
    };

    let skip_intro = {
        let onboarding_dismissed = onboarding_dismissed.clone();
        Callback::from(move |_| {
            if should_dismiss_onboarding(false, true) {
                onboarding_dismissed.set(true);
                persist_dismissed_preference(ONBOARDING_KEY);
            }
        })
    };

    let dismiss_alignment_hint = {
        let alignment_hint_dismissed = alignment_hint_dismissed.clone();
        Callback::from(move |_| {
            alignment_hint_dismissed.set(true);
            persist_dismissed_preference(ALIGNMENT_HINT_KEY);
        })
    };

    let open_guide = {
        let guide_open = guide_open.clone();
        Callback::from(move |_| guide_open.set(true))
    };

    let close_guide = {
        let guide_open = guide_open.clone();
        let setup_guide_trigger_ref = setup_guide_trigger_ref.clone();
        Callback::from(move |_| {
            guide_open.set(false);
            if let Some(trigger) = setup_guide_trigger_ref.cast::<HtmlElement>() {
                let _ = trigger.focus();
            }
        })
    };

    let remove_current = {
        let current = current.clone();
        let history = history.clone();
        let overlay_hidden = overlay_hidden.clone();
        let position_locked = position_locked.clone();
        let position_locked_ref = position_locked_ref.clone();
        let drag_state = drag_state.clone();
        Callback::from(move |_| {
            let Some(project) = (*current).clone() else {
                return;
            };
            let mut next = (*history).clone();
            next.retain(|item| item.id != project.id);
            current.set(next.first().cloned());
            history.set(next);
            overlay_hidden.set(false);
            position_locked.set(false);
            *position_locked_ref.borrow_mut() = false;
            *drag_state.borrow_mut() = None;
            spawn_local(async move {
                let _ = delete_project(project.id).await;
            });
        })
    };

    let overlay = current.as_ref().filter(|_| !*overlay_hidden).map(|project| {
        let flip_x = if project.flipped { -1.0 } else { 1.0 };
        let style = format!(
            "opacity:{};transform:translate(calc(-50% + {}px),calc(-50% + {}px)) rotate({}deg) scale({},{})",
            project.opacity, project.x, project.y, project.rotation, project.scale * flip_x, project.scale
        );
        html! { <img class="reference" src={project.image.clone()} alt="Reference overlay" {style} draggable="false" /> }
    });

    let intro_visible = should_show_intro(
        *history_loaded,
        *onboarding_dismissed,
        current.is_some(),
        !history.is_empty(),
    );

    html! {
        <main class="app-shell">
            <section class="viewport">
                <input
                    ref={file_input_ref}
                    class="visually-hidden"
                    type="file"
                    accept="image/*"
                    onchange={on_file}
                />
                <video ref={video_ref} autoplay=true playsinline=true muted=true></video>
                if !intro_visible {
                    {overlay.unwrap_or_else(|| html! {
                        <div class="empty-state">
                            <div class="empty-mark">{"＋"}</div>
                            <strong>{"Choose a reference photo"}</strong>
                            <p>{"It will appear over the live camera view so you can align and trace it."}</p>
                        </div>
                    })}
                }
                if current.is_some() && !*overlay_hidden && !*position_locked {
                    <div
                        class="gesture-surface"
                        role="application"
                        aria-label="Drag to position reference image"
                        onpointerdown={on_pointer_down}
                        onpointermove={on_pointer_move}
                        onpointerup={on_pointer_up.clone()}
                        onpointercancel={on_pointer_up}
                        ontouchstart={on_touch_start}
                        ontouchmove={on_touch_move}
                        ontouchend={on_touch_end.clone()}
                        ontouchcancel={on_touch_end}
                    />
                }
                if !intro_visible && !*camera_on {
                    <div class={classes!("camera-prompt", current.is_some().then_some("with-reference"))}>
                        <strong>{"Camera is off"}</strong>
                        <p>{"Start it when you’re ready to view the paper. Permission is only requested after you tap."}</p>
                        <button class="primary" onclick={start_camera}>{"Start camera"}</button>
                        if let Some(message) = camera_error.as_ref() {
                            <p class="error">{message}</p>
                        }
                    </div>
                }
                if current.is_some() && !*overlay_hidden && *position_locked {
                    <div class={classes!("view-hint", (*position_locked).then_some("locked"))} role="status">
                        <span></span>
                        {"Position locked on screen"}
                    </div>
                }

                if !intro_visible {
                    <button
                        ref={setup_guide_trigger_ref.clone()}
                        class="setup-guide-trigger"
                        onclick={open_guide}
                    >{"Setup guide"}</button>

                    <section class="control-dock">
                    <div class="action-row">
                        <button class="primary" onclick={choose_photo.clone()} disabled={*busy}>
                            {if *busy { "Preparing…" } else if current.is_some() { "Replace photo" } else { "Choose photo" }}
                        </button>
                        <button onclick={toggle_overlay} disabled={current.is_none()}>
                            {if *overlay_hidden { "Show" } else { "Hide" }}
                        </button>
                        <button onclick={flip} disabled={current.is_none() || *position_locked}>{"Flip"}</button>
                        <button onclick={reset} disabled={current.is_none() || *position_locked}>{"Reset"}</button>
                    </div>

                    if let Some(message) = import_error.as_ref() {
                        <p class="import-error" role="alert">{message}</p>
                    }

                    if current.is_some() && *camera_on && !*position_locked && !*alignment_hint_dismissed {
                        <aside class="context-hint" aria-label="Alignment tip">
                            <p><strong>{"Align the overlay"}</strong>{" Drag on the camera view, then adjust size and opacity. When it lines up, tap Lock position below."}</p>
                            <button aria-label="Dismiss alignment tip" onclick={dismiss_alignment_hint.clone()}> {"×"} </button>
                        </aside>
                    }

                    if current.is_some() {
                        <div class={classes!("lock-row", (*position_locked).then_some("is-locked"))}>
                            <div class="lock-copy">
                                <strong>{if *position_locked { "Position locked" } else { "Ready to trace?" }}</strong>
                                <span>{"Locks the image on screen—not the phone. Keep your phone steady while tracing."}</span>
                            </div>
                            <button
                                class={classes!((!*position_locked).then_some("primary"), "lock-button")}
                                onclick={toggle_position_lock}
                                aria-pressed={position_locked.to_string()}
                            >
                                {if *position_locked { "Unlock" } else { "Lock position" }}
                            </button>
                        </div>
                    }

                    if let Some(project) = current.as_ref() {
                        <div class="sliders">
                            <label>
                                <span>{"Opacity"}</span>
                                <input type="range" min="0.05" max="1" step="0.01" value={project.opacity.to_string()} oninput={set_number("opacity")} />
                                <output>{format!("{}%", (project.opacity * 100.0).round())}</output>
                            </label>
                            <label>
                                <span>{"Scale"}</span>
                                <input type="range" min="0.2" max="3" step="0.05" value={project.scale.to_string()} oninput={set_number("scale")} disabled={*position_locked} />
                                <output>{format!("{:.1}×", project.scale)}</output>
                            </label>
                            <label>
                                <span>{"Rotate"}</span>
                                <input type="range" min="-180" max="180" step="1" value={project.rotation.to_string()} oninput={set_number("rotation")} disabled={*position_locked} />
                                <output>{format!("{}°", project.rotation.round())}</output>
                            </label>
                        </div>
                    }

                    if !history.is_empty() {
                        <div class="history-row">
                            <div class="history-strip">
                                {for history.iter().map(|project| {
                                    let selected = current.as_ref().is_some_and(|item| item.id == project.id);
                                    let item = project.clone();
                                    let current = current.clone();
                                    let overlay_hidden = overlay_hidden.clone();
                                    let position_locked = position_locked.clone();
                                    let position_locked_ref = position_locked_ref.clone();
                                    let drag_state = drag_state.clone();
                                    html! {
                                        <button class={classes!("history-item", selected.then_some("selected"))} onclick={Callback::from(move |_| {
                                            current.set(Some(item.clone()));
                                            overlay_hidden.set(false);
                                            if !selected {
                                                position_locked.set(false);
                                                *position_locked_ref.borrow_mut() = false;
                                                *drag_state.borrow_mut() = None;
                                            }
                                        })}>
                                            <img src={project.image.clone()} alt="Saved reference" />
                                        </button>
                                    }
                                })}
                            </div>
                            <button class="danger-link" onclick={remove_current} disabled={current.is_none()}>{"Delete"}</button>
                        </div>
                    }
                    </section>
                }

                if intro_visible {
                    <section class="intro-screen" aria-labelledby="intro-title">
                        <div class="intro-content">
                            <p class="eyebrow">{"Camera Trace"}</p>
                            <h1 id="intro-title">{"Trace a photo onto paper"}</h1>
                            <p class="intro-lede">{"See your reference over the live camera view, then draw on paper while watching your phone screen."}</p>
                            <p class="intro-clarification">{"The image appears on your screen—it isn’t projected onto the paper."}</p>
                            <SetupSteps />
                            if let Some(message) = import_error.as_ref() {
                                <p class="intro-error" role="alert">{message}</p>
                            }
                            <div class="intro-actions">
                                <button class="primary" onclick={choose_photo.clone()} disabled={*busy}>
                                    {if *busy { "Preparing…" } else { "Choose photo" }}
                                </button>
                                <button class="secondary-action" onclick={skip_intro}>{"Skip intro"}</button>
                            </div>
                            <p class="privacy-note">{"Free to use. No account needed. Photos are processed and saved in this browser."}</p>
                        </div>
                    </section>
                }

                if *guide_open {
                    <GuideSheet on_close={close_guide} close_ref={guide_close_ref.clone()} />
                }
            </section>

        </main>
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}

#[cfg(test)]
mod tests {
    use super::{Project, should_dismiss_onboarding, should_show_intro, stored_value_is_dismissed};

    fn project() -> Project {
        Project {
            id: "test".into(),
            image: "data:image/png;base64,test".into(),
            opacity: 0.5,
            rotation: 24.0,
            scale: 1.4,
            x: 18.0,
            y: -9.0,
            flipped: true,
            updated_at: 0.0,
        }
    }

    #[test]
    fn locked_project_rejects_every_alignment_mutation() {
        let mut project = project();
        let original = project.clone();

        assert!(!project.move_to(100.0, 200.0, true));
        assert!(!project.set_number("scale", 2.5, true));
        assert!(!project.set_number("rotation", 90.0, true));
        assert!(!project.flip(true));
        assert!(!project.reset(true));
        assert_eq!(project, original);
    }

    #[test]
    fn opacity_remains_editable_while_position_is_locked() {
        let mut project = project();
        let transform = (
            project.x,
            project.y,
            project.scale,
            project.rotation,
            project.flipped,
        );

        assert!(project.set_number("opacity", 0.8, true));
        assert_eq!(project.opacity, 0.8);
        assert_eq!(
            (
                project.x,
                project.y,
                project.scale,
                project.rotation,
                project.flipped,
            ),
            transform
        );
    }

    #[test]
    fn unlocked_project_accepts_alignment_mutations() {
        let mut project = project();

        assert!(project.move_to(100.0, 200.0, false));
        assert!(project.set_number("scale", 2.5, false));
        assert!(project.set_number("rotation", 90.0, false));
        assert!(project.flip(false));
        assert_eq!((project.x, project.y), (100.0, 200.0));
        assert_eq!((project.scale, project.rotation), (2.5, 90.0));
        assert!(!project.flipped);
    }

    #[test]
    fn fresh_visit_shows_intro_only_after_project_storage_has_loaded() {
        assert!(!should_show_intro(false, false, false, false));
        assert!(should_show_intro(true, false, false, false));
    }

    #[test]
    fn returning_or_active_projects_do_not_show_intro() {
        assert!(!should_show_intro(true, true, false, false));
        assert!(!should_show_intro(true, false, true, false));
        assert!(!should_show_intro(true, false, false, true));
    }

    #[test]
    fn only_skip_or_successful_import_dismisses_onboarding() {
        assert!(should_dismiss_onboarding(false, true));
        assert!(should_dismiss_onboarding(true, false));
        assert!(!should_dismiss_onboarding(false, false));
    }

    #[test]
    fn unavailable_or_corrupted_preference_falls_back_to_not_dismissed() {
        assert!(!stored_value_is_dismissed(None));
        assert!(!stored_value_is_dismissed(Some("")));
        assert!(!stored_value_is_dismissed(Some("true")));
        assert!(!stored_value_is_dismissed(Some("{bad json")));
        assert!(stored_value_is_dismissed(Some("dismissed")));
    }
}
