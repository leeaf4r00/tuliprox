use crate::{app::components::{custom_dialog::CustomDialog, input::Input}, i18n::use_translation,
    services::{get_base_href, request_post, Encoding}};
use serde::Deserialize;
use shared::utils::concat_path_leading_slash;
use yew::prelude::*;

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct DirectoryListing {
    path: String,
    parent: Option<String>,
    directories: Vec<String>,
}

#[derive(Properties, PartialEq)]
pub struct DownloadDirectoryFieldProps {
    pub value: String,
    pub on_change: Callback<String>,
}

#[component]
pub fn DownloadDirectoryField(props: &DownloadDirectoryFieldProps) -> Html {
    let translate = use_translation();
    let open = use_state(|| false);
    let path = use_state(|| props.value.clone());
    let listing = use_state(|| None::<DirectoryListing>);
    let error = use_state(|| false);
    let loading = use_state(|| false);
    let generation = use_mut_ref(|| 0_u64);
    {
        let listing = listing.clone();
        let error = error.clone();
        let loading = loading.clone();
        let generation = generation.clone();
        use_effect_with((*open, (*path).clone()), move |(is_open, path)| {
            *generation.borrow_mut() += 1;
            let request_generation = *generation.borrow();
            if *is_open {
                loading.set(true);
                error.set(false);
                listing.set(None);
                let path = path.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let result = request_post::<&String, DirectoryListing>(
                        &concat_path_leading_slash(&get_base_href(), "api/v1/config/directories"),
                        &path, None, Some(Encoding::Json)).await;
                    if *generation.borrow() == request_generation {
                        loading.set(false);
                        match result {
                            Ok(Some(value)) => listing.set(Some(value)),
                            _ => error.set(true),
                        }
                    }
                });
            }
            || ()
        });
    }
    let close = { let open = open.clone(); Callback::from(move |()| open.set(false)) };
    let directory_label = translate.t("LABEL.DIRECTORY");
    html! {
        <>
            <Input name="download-directory" label={Some(directory_label.clone())}
                value={props.value.clone()} on_change={Some(props.on_change.clone())}
                onclick={Some({let open = open.clone(); let path = path.clone(); let value = props.value.clone(); Callback::from(move |_| {path.set(value.clone()); open.set(true);})})} />
            {if *open {html! {
                <CustomDialog class={Some("tp__content-dialog".to_string())} aria_label={Some(directory_label.clone())}
                    on_close={Some(close.clone())} close_on_backdrop_click=true>
                    <h2>{directory_label}</h2>
                    <div style="min-width:300px;max-width:70vw;max-height:50vh;overflow:auto">
                        {if *loading {html!{<p>{translate.t("DIRECTORY_PICKER.LOADING")}</p>}} else {html!{}}}
                        {if *error {html!{<p role="alert">{translate.t("DIRECTORY_PICKER.ERROR")}</p>}} else {html!{}}}
                        {if let Some(value) = listing.as_ref() {html!{
                            <>
                                <p style="overflow-wrap:anywhere">{value.path.clone()}</p>
                                {if let Some(parent) = &value.parent {html!{
                                    <button type="button" class="tp__text-button" onclick={{let path = path.clone(); let parent = parent.clone(); Callback::from(move |_| path.set(parent.clone()))}}>{translate.t("DIRECTORY_PICKER.UP")}</button>
                                }} else {html!{}}}
                                {for value.directories.iter().map(|directory| {
                                    let target = directory.clone(); let path = path.clone();
                                    html!{<button type="button" class="tp__text-button" style="display:block;width:100%;text-align:left;padding:10px" onclick={Callback::from(move |_| path.set(target.clone()))}>{directory.rsplit('/').next().unwrap_or(directory)}</button>}
                                })}
                                <button type="button" class="tp__text-button" onclick={{let on_change = props.on_change.clone(); let selected = value.path.clone(); let close = close.clone(); Callback::from(move |_| {on_change.emit(selected.clone()); close.emit(());})}}>{translate.t("DIRECTORY_PICKER.SELECT")}</button>
                            </>
                        }} else {html!{}}}
                        {if *error {html!{<button type="button" class="tp__text-button" onclick={{let path = path.clone(); Callback::from(move |_| path.set("/app".to_string()))}}>{translate.t("DIRECTORY_PICKER.HOME")}</button>}} else {html!{}}}
                    </div>
                    <button type="button" class="tp__text-button" onclick={Callback::from(move |_| close.emit(()))}>{translate.t("DIRECTORY_PICKER.CANCEL")}</button>
                </CustomDialog>
            }} else {html!{}}}
        </>
    }
}
