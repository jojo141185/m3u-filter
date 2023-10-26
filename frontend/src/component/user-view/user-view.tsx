import React, {useCallback, useEffect, useState} from "react";
import './user-view.scss';
import ServerConfig, {TargetUser} from "../../model/server-config";
import {getIconByName} from "../../icons/icons";
import TextGenerator from "../../utils/text-generator";
import {useSnackbar} from "notistack";
import {useServices} from "../../provider/service-provider";
import ConfigUtils from "../../utils/config-utils";

interface UserViewProps {
    config: ServerConfig;
}

export default function UserView(props: UserViewProps) {
    const {config} = props;
    const services = useServices();
    const {enqueueSnackbar/*, closeSnackbar*/} = useSnackbar();
    const [targets, setTargets] = useState<TargetUser[]>([]);
    useEffect(() => {
        if (config) {
            const target_names = ConfigUtils.getTargetNames(config);
            const missing = config?.user.filter(target => !target_names.includes(target.target));
            const result: TargetUser[] = target_names?.map(name => ({
                src: true,
                target: name,
                credentials: config.user.find(t => t.target === name)?.credentials || []
            } as any));
            missing?.forEach(target => {
                result.push({src: false, target: target.target, credentials: target.credentials} as any);
            });
            setTargets(result || []);
        }
    }, [config])

    const handleUserAdd = useCallback((evt: any) => {
        const target_name = evt.target.dataset.target;
        const target = targets.find(target => target.target === target_name);
        if (target) {
            const usernameExists = (uname: string): boolean => {
                for (const target of targets) {
                    if (target.credentials.find(c => c.username === uname)) {
                        return true;
                    }
                }
                return false;
            };
            let cnt = 0;
            let username = TextGenerator.generateUsername().toLowerCase();
            while (usernameExists(username)) {
                username = TextGenerator.generateUsername().toLowerCase();
                cnt++;
                if (cnt > 1000) {
                    username = "";
                    break;
                }
            }
            target.credentials.push({username, password: TextGenerator.generatePassword(), token: TextGenerator.generatePassword()});
            setTargets([...targets]);
        }
    }, [targets]);

    const handleUserRemove = useCallback((evt: any) => {
        const idx = evt.target.dataset.idx;
        const target_name = evt.target.dataset.target;
        const target = targets.find(target => target.target === target_name);
        if (target) {
            target.credentials.splice(idx, 1);
            setTargets([...targets]);
        }
    }, [targets]);

    const handleValueChange = useCallback((evt: any) => {
        const target_name = evt.target.dataset.target;
        const target = targets.find(target => target.target === target_name);
        if (target) {
            const idx = evt.target.dataset.idx;
            const field: any = evt.target.dataset.field;
            if (field === 'username') {
                target.credentials[idx].username = evt.target.value;
            } else if (field === 'password') {
                target.credentials[idx].password = evt.target.value;
            } else if (field === 'token') {
                target.credentials[idx].token = evt.target.value;
            }
        }
    }, [targets]);

    const handleSave = useCallback(() => {
        const usernames: any = {};
        for(const target of targets) {
            for (const user of target.credentials) {
                if (!user.username?.trim().length) {
                    enqueueSnackbar("Username empty!", {variant: 'error'});
                    return;
                }
                if (usernames[user.username]) {
                    enqueueSnackbar("Duplicate Username! " + user.username, {variant: 'error'});
                    return;
                }
                usernames[user.username] = true;
            }
        }
        const targetUser = targets.map(t => {
            t.credentials.forEach(c => {
                c.username = c.username.trim();
                c.password = c.password.trim();
                c.token = c.token?.trim();
            })
            return {target: t.target, credentials: t.credentials}
        });
        services.config().saveTargetUser(targetUser).subscribe({
            next: () => enqueueSnackbar("User saved!", {variant: 'success'}),
            error: (err) => enqueueSnackbar("Failed to save user!", {variant: 'error'})
        });
    }, [targets, services, enqueueSnackbar]);

    return <div className={'user'}>

        <div className={'user__toolbar'}><label>User</label><button onClick={handleSave}>Save</button></div>
        <div className={'user__content'}>
        <div className={'user__content-targets'}>
            {
                targets?.map(target => <div key={target.target}
                                            className={'user__target'}>
                    <div className={'user__target-target'}>
                        <label className={(target as any).src ? '' : 'target-not-exists'}>{target.target}</label>
                        <div className={'user__target-target-toolbar'}>
                            <button data-target={target.target}
                                    onClick={handleUserAdd}>{getIconByName('PersonAdd')}</button>
                        </div>
                    </div>

                    <div className={'user__target-user-table'}>
                        <div className={'user__target-user-row user__target-user-table-header'}>
                            <div className={'user__target-user-col'}><label>Username</label></div>
                            <div className={'user__target-user-col'}><label>Password</label></div>
                            <div className={'user__target-user-col'}><label>Token</label></div>
                            <div className={'user__target-user-col'}></div>
                        </div>
                        {target.credentials.map((usr, idx) =>
                            <div key={'credential' + idx} className={'user__target-user-row'}>
                                <div className={'user__target-user-col'}>
                                    <div className={'user__target-user-col-label'}><label>Username</label></div>
                                    <input data-target={target.target} data-idx={idx} defaultValue={usr.username}
                                           key={usr.username}
                                           data-field={'username'} onChange={handleValueChange}></input>
                                </div>
                                <div className={'user__target-user-col'}>
                                    <div className={'user__target-user-col-label'}><label>Password</label></div>
                                    <input defaultValue={usr.password} key={usr.password} data-field={'password'}
                                           onChange={handleValueChange}></input>
                                </div>
                                <div className={'user__target-user-col'}>
                                    <div className={'user__target-user-col-label'}><label>Token</label></div>
                                    <input defaultValue={usr.token} key={usr.token} data-field={'token'}
                                           onChange={handleValueChange}></input>
                                </div>
                                <div className={'user__target-user-col toolbar'}>
                                <span data-target={target.target} data-idx={idx} onClick={handleUserRemove}>
                                    {getIconByName('PersonRemove')}
                                </span>
                                </div>
                            </div>
                        )}
                    </div>
                </div>)}
            </div>
        </div>
    </div>
}