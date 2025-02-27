import React from "react";
import './toolbar.scss';
import useTranslator from "../../hook/use-translator";

interface ToolbarProps {
  onDownload: () => void;
}

export default function Toolbar(props: ToolbarProps) {
    const translate = useTranslator();
    return <div className={'toolbar'}>
        <button data-tooltip='LABEL.SAVE' onClick={props.onDownload}>
            {translate('LABEL.SAVE')}
        </button>
    </div>
}