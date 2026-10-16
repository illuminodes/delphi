let THREE = null;
let RoomEnvironment = null;

export class CarvedSlab {
    static FIELD_OF_VIEW = 18;
    static LIGHT_DISTANCE = 4000;
    static SHADOW_MAP = 4096;

    static async create(canvas) {
        if (THREE === null) {
            THREE = await import('three');
            ({ RoomEnvironment } = await import('three/addons/environments/RoomEnvironment.js'));
        }
        return new CarvedSlab(canvas);
    }

    static debug() {
        const query = new URLSearchParams(window.location.search);
        return {
            zoom: Number(query.get('zoom') ?? 1),
            focusX: Number(query.get('fx') ?? 0.12),
            focusY: Number(query.get('fy') ?? 0.08),
            view: query.get('view') ?? 'full',
            environment: query.has('env') ? Number(query.get('env')) : null,
            clearcoat: query.get('cc') !== '0',
        };
    }

    constructor(canvas) {
        this.debug = CarvedSlab.debug();
        console.info('carved slab debug', this.debug);
        this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
        this.renderer.toneMapping = THREE.NeutralToneMapping;
        this.renderer.shadowMap.enabled = true;
        this.renderer.shadowMap.type = THREE.PCFSoftShadowMap;
        this.scene = new THREE.Scene();
        const environment = new THREE.PMREMGenerator(this.renderer);
        this.scene.environment = environment.fromScene(new RoomEnvironment(), 0.04).texture;
        environment.dispose();
        this.camera = new THREE.PerspectiveCamera(CarvedSlab.FIELD_OF_VIEW, 1, 1, 100000);
        this.sun = new THREE.DirectionalLight(0xfff4e6, 2.4);
        this.sun.castShadow = true;
        this.sun.shadow.mapSize.set(CarvedSlab.SHADOW_MAP, CarvedSlab.SHADOW_MAP);
        this.sun.shadow.bias = -0.0002;
        this.sun.shadow.normalBias = 0.15;
        this.sun.shadow.radius = 1.5;
        this.scene.add(this.sun, this.sun.target);
        this.geometry = null;
        this.mesh = null;
    }

    frame(width, height, pixelRatio) {
        this.renderer.setPixelRatio(pixelRatio);
        this.renderer.setSize(width, height, false);
        this.camera.aspect = width / height;
        this.camera.position.set(0, 0, height / 2 / Math.tan(THREE.MathUtils.degToRad(CarvedSlab.FIELD_OF_VIEW / 2)));
        this.camera.far = this.camera.position.z * 2;
        if (this.debug.zoom > 1) {
            const [w, h] = [width / this.debug.zoom, height / this.debug.zoom];
            const x = Math.min(Math.max(this.debug.focusX * width - w / 2, 0), width - w);
            const y = Math.min(Math.max(this.debug.focusY * height - h / 2, 0), height - h);
            this.camera.setViewOffset(width, height, x, y, w, h);
        } else {
            this.camera.clearViewOffset();
        }
        this.camera.updateProjectionMatrix();
        const shadow = this.sun.shadow.camera;
        shadow.left = -width / 2;
        shadow.right = width / 2;
        shadow.top = height / 2;
        shadow.bottom = -height / 2;
        shadow.far = CarvedSlab.LIGHT_DISTANCE * 2;
        shadow.updateProjectionMatrix();
    }

    carve(width, height, columns, rows, heights) {
        this.clear();
        this.geometry = new THREE.PlaneGeometry(width, height, columns - 1, rows - 1);
        const position = this.geometry.attributes.position;
        for (let index = 0; index < position.count; index++) {
            position.setZ(index, -heights[index]);
        }
        this.geometry.computeVertexNormals();
    }

    finish(mapWidth, mapHeight, albedo, normals, surface, environment) {
        const finish = this.texture(surface, mapWidth, mapHeight, THREE.NoColorSpace);
        const view = this.debug.view;
        if (this.debug.environment !== null) {
            environment = this.debug.environment;
        }
        if (view === 'diag') {
            const normalMap = this.texture(normals, mapWidth, mapHeight, THREE.NoColorSpace);
            const albedoMap = this.texture(albedo, mapWidth, mapHeight, THREE.SRGBColorSpace);
            const full = (overrides) => new THREE.MeshPhysicalMaterial({ map: albedoMap, normalMap, normalMapType: THREE.ObjectSpaceNormalMap, roughnessMap: finish, metalnessMap: finish, clearcoatMap: finish, metalness: 1, roughness: 1, clearcoat: 1, clearcoatRoughness: 0.06, envMapIntensity: environment, ...overrides });
            this.diagMaterials = [
                full({}),
                full({ clearcoat: 0 }),
                full({ envMapIntensity: 0 }),
                full({ map: null, color: new THREE.Color(0x5a5a5a) }),
            ];
            this.mesh = new THREE.Mesh(this.geometry, this.diagMaterials[0]);
            this.mesh.castShadow = true;
            this.mesh.receiveShadow = true;
            this.scene.add(this.mesh);
            window.__slab = this;
            return;
        }
        if (view === 'normals' || view === 'geonormals') {
            const material = new THREE.MeshNormalMaterial(view === 'normals' ? {
                normalMap: this.texture(normals, mapWidth, mapHeight, THREE.NoColorSpace),
                normalMapType: THREE.ObjectSpaceNormalMap,
            } : {});
            this.mesh = new THREE.Mesh(this.geometry, material);
            this.scene.add(this.mesh);
            return;
        }
        const clay = view === 'clay' || view === 'claygeo';
        const material = new THREE.MeshPhysicalMaterial({
            map: this.texture(albedo, mapWidth, mapHeight, THREE.SRGBColorSpace),
            normalMap: this.texture(normals, mapWidth, mapHeight, THREE.NoColorSpace),
            normalMapType: THREE.ObjectSpaceNormalMap,
            roughnessMap: finish,
            metalnessMap: finish,
            clearcoatMap: finish,
            metalness: 1,
            roughness: 1,
            clearcoat: 1,
            clearcoatRoughness: 0.06,
            envMapIntensity: environment,
        });
        if (clay) {
            material.map = null;
            material.color.set(0x8a8a8a);
        }
        if (view === 'claygeo' || view === 'nonormal') {
            material.normalMap = null;
        }
        if (!this.debug.clearcoat || clay) {
            material.clearcoat = 0;
        }
        this.mesh = new THREE.Mesh(this.geometry, material);
        this.mesh.castShadow = true;
        this.mesh.receiveShadow = true;
        this.scene.add(this.mesh);
    }

    texture(pixels, width, height, colorSpace) {
        const texture = new THREE.DataTexture(new Uint8Array(pixels), width, height, THREE.RGBAFormat);
        texture.colorSpace = colorSpace;
        texture.repeat.set(1, -1);
        texture.offset.set(0, 1);
        texture.magFilter = THREE.LinearFilter;
        texture.minFilter = THREE.LinearMipmapLinearFilter;
        texture.generateMipmaps = true;
        texture.anisotropy = this.renderer.capabilities.getMaxAnisotropy();
        texture.needsUpdate = true;
        return texture;
    }

    light(x, y, z) {
        this.sun.position.set(x, y, z).multiplyScalar(CarvedSlab.LIGHT_DISTANCE);
    }

    render() {
        if (this.debug.view !== 'diag' || !this.diagMaterials) {
            this.renderer.render(this.scene, this.camera);
            return;
        }
        const size = this.renderer.getSize(new THREE.Vector2());
        const [halfW, halfH] = [size.x / 2, size.y / 2];
        const zoom = this.debug.zoom > 1 ? this.debug.zoom : 5;
        const [w, h] = [size.x / zoom, size.y / zoom];
        const x = Math.min(Math.max(this.debug.focusX * size.x - w / 2, 0), size.x - w);
        const y = Math.min(Math.max(this.debug.focusY * size.y - h / 2, 0), size.y - h);
        this.camera.setViewOffset(size.x, size.y, x, y, w, h);
        this.camera.updateProjectionMatrix();
        this.renderer.setScissorTest(true);
        this.diagMaterials.forEach((material, index) => {
            const [left, bottom] = [(index % 2) * halfW, (index < 2 ? 1 : 0) * halfH];
            this.renderer.setViewport(left, bottom, halfW, halfH);
            this.renderer.setScissor(left, bottom, halfW, halfH);
            this.mesh.material = material;
            this.renderer.render(this.scene, this.camera);
        });
        this.renderer.setScissorTest(false);
        this.renderer.setViewport(0, 0, size.x, size.y);
        this.mesh.material = this.diagMaterials[0];
    }

    clear() {
        if (this.mesh !== null) {
            this.scene.remove(this.mesh);
            const material = this.mesh.material;
            for (const map of new Set([material.map, material.normalMap, material.roughnessMap])) {
                map?.dispose();
            }
            material.dispose();
            this.mesh = null;
        }
        if (this.geometry !== null) {
            this.geometry.dispose();
            this.geometry = null;
        }
    }

    dispose() {
        this.clear();
        this.scene.environment.dispose();
        this.renderer.dispose();
    }
}
