
// Declaration of all textures

layout(set=0, binding = 1) uniform sampler2D aces_reach_m_table_0Sampler; 
layout(set=0, binding = 2) uniform sampler2D aces_gamut_cusp_table_0Sampler; 

// Declaration of all helper methods

float aces_reach_m_table_0_sample(float h)
{
  float i_base = floor(h);
  float i_lo = i_base + float(1);
  float i_hi = i_lo + 1.0;
  float lo = texture(aces_reach_m_table_0Sampler, vec2((i_lo + 0.5) / float (363), 0.0)).r;
  float hi = texture(aces_reach_m_table_0Sampler, vec2((i_hi + 0.5) / float (363), 0.5)).r;
  float t = h - i_base;
  return mix(lo, hi, t);
}
float aces_tonescale_fwd0(float J)
{
  float A = 0.0323680267 * pow(abs(J) * 0.00999999978, 0.879464149);
  float Y = pow(( 27.1299992 * A) / (1.0f - A), 2.3809523809523809);
  float f = 5.08923578 * pow(Y / (Y + 2.6881951446344488), 1.14999998);
  float Y_ts = max(0.0, f * f / (f + 0.0399999991));
  float F_L_Y = pow(0.79370057210326195 * Y_ts, 0.42);
  float J_ts = 100. * pow((F_L_Y / ( 27.1299992 + F_L_Y)) * 30.8946857, 1.13705599);
  return sign(J) * J_ts;
}
float aces_toe_fwd0(float x, float limit, float k1_in, float k2_in)
{
  float k2 = max(k2_in, 0.001);
  float k1 = sqrt(k1_in * k1_in + k2 * k2);
  float k3 = (limit + k1) / (limit + k2);
  return (x > limit) ? x : 0.5 * (k3 * x - k1 + sqrt((k3 * x - k1) * (k3 * x - k1) + 4.0 * k2 * k3 * x));
}
const float aces_gamut_cusp_table_0_hues_array[363] = float[363](-1.00979614, 0., 1.01336622, 2.02673244, 3.04009867, 4.05346489, 5.06683111, 6.08019733, 7.09356356, 8.10692978, 9.12029648, 10.1336622, 11.147028, 12.1603947, 13.1737614, 14.1871271, 15.2004929, 16.2138596, 17.2272263, 18.240593, 19.2539577, 20.2673244, 21.2806911, 22.2940559, 23.3074226, 24.0521469, 24.7968731, 25.5415974, 26.5578537, 27.57411, 28.5903645, 29.6066208, 30.6228771, 31.6391335, 32.6553879, 33.6716461, 34.6879005, 35.7041588, 36.7204132, 37.7366676, 38.7529259, 39.7691803, 40.7854385, 41.801693, 42.8179474, 43.8342056, 44.8504601, 45.8667183, 46.8829727, 47.8992271, 48.9154854, 49.9317398, 50.947998, 51.9642525, 52.9805069, 53.9967651, 55.0130196, 56.0292778, 57.0455322, 58.0617905, 59.0780449, 60.0942993, 61.1105576, 62.126812, 63.1430702, 64.1593246, 65.1755829, 66.1918335, 67.2080917, 68.22435, 69.2406006, 70.2568588, 71.2731171, 72.2893753, 73.3056259, 74.3218842, 75.3381424, 76.354393, 77.3706512, 78.3869095, 79.4031677, 80.4194183, 81.4356766, 82.4519348, 83.4681854, 84.4844437, 85.5007019, 86.5169525, 87.5332108, 88.549469, 89.5657272, 90.5819778, 91.5982361, 92.6144943, 93.6307449, 94.6470032, 95.6632614, 96.679512, 97.6957703, 98.7120285, 99.7282867, 100.744537, 101.760796, 102.777054, 103.793304, 104.809563, 105.825813, 106.226929, 107.216499, 108.20607, 109.195641, 110.185211, 111.174782, 112.164352, 113.153923, 114.143501, 115.133072, 116.122643, 117.112213, 118.101784, 119.091354, 120.080925, 121.070496, 122.060066, 123.049637, 124.039207, 125.028778, 126.018349, 127.007919, 127.997498, 128.987061, 129.976639, 130.966202, 131.95578, 132.945343, 133.934921, 134.9245, 135.914062, 136.903641, 137.893204, 138.882782, 139.872345, 140.861923, 141.851486, 142.841064, 143.830627, 144.820206, 145.809769, 146.799347, 147.954407, 148.980301, 150.00618, 151.032074, 152.057953, 153.083847, 154.109726, 155.13562, 156.161499, 157.187393, 158.213272, 159.239166, 160.265045, 161.290939, 162.316818, 163.342712, 164.368591, 165.394485, 166.42038, 167.446259, 168.472153, 169.498032, 170.523926, 171.549805, 172.575699, 173.601578, 174.627472, 175.653351, 176.679245, 177.705124, 178.731018, 179.756897, 180.782791, 181.808685, 182.834564, 183.860458, 184.886337, 185.912231, 186.93811, 187.964005, 188.989883, 190.015778, 191.041656, 192.067551, 192.186447, 193.189835, 194.193222, 195.196609, 196.199997, 197.203384, 198.206787, 199.210175, 200.213562, 201.216949, 202.220337, 203.223724, 204.227112, 205.230499, 206.233887, 207.237274, 208.240662, 209.244064, 210.247452, 211.250839, 212.254227, 213.257614, 214.261002, 215.264389, 216.267776, 217.271164, 218.274551, 219.277954, 220.281342, 221.284729, 222.288116, 223.291504, 224.294891, 225.298279, 226.301666, 227.305054, 228.308441, 229.311829, 230.315231, 231.318619, 232.322006, 233.325394, 234.328781, 235.332169, 236.335556, 237.338943, 238.342331, 239.345718, 240.349121, 241.352509, 242.355896, 243.359283, 244.362671, 245.366058, 246.369446, 247.372833, 248.376221, 249.379608, 250.382996, 251.386398, 252.389786, 253.393173, 254.396561, 255.399948, 256.40332, 257.406738, 258.410126, 259.413513, 260.416901, 261.420288, 262.423676, 263.427063, 264.43045, 265.433838, 266.437225, 267.440613, 268.324341, 269.208069, 270.212555, 271.217072, 272.221558, 273.226074, 274.23056, 275.235077, 276.239563, 277.244049, 278.248566, 279.253052, 280.257568, 281.262054, 282.266571, 283.271057, 284.275543, 285.28006, 286.284546, 287.289062, 288.293549, 289.298065, 290.302551, 291.307037, 292.311554, 293.31604, 294.320557, 295.325043, 296.329559, 297.334045, 298.338531, 299.343048, 300.347534, 301.352051, 302.356537, 303.361053, 304.36554, 305.370026, 306.374542, 307.379028, 308.383545, 309.388031, 310.392548, 311.397034, 312.40152, 313.406036, 314.410522, 315.415039, 316.419525, 317.424042, 318.428528, 319.433014, 320.437531, 321.442017, 322.446533, 323.451019, 324.455536, 325.460022, 326.464508, 327.469025, 328.473511, 329.478027, 330.482513, 331.48703, 332.491516, 333.496033, 334.500519, 335.505005, 336.509521, 337.514008, 338.518524, 339.52301, 340.527527, 341.532013, 341.823578, 342.833374, 343.84317, 344.852997, 345.862793, 346.872589, 347.882385, 348.892181, 349.901978, 350.911804, 351.9216, 352.931396, 353.941193, 354.950989, 355.960785, 356.970612, 357.980408, 358.990204, 360., 361.013367);
vec3 aces_gamut_cusp_table_0_sample(float h)
{
  int i = int(h) + 1;
  int i_lo = int(max(float(0), float(i + -1)));
  int i_hi = int(min(float(361), float(i + 2)));
  while (i_lo + 1 < i_hi)
  {
    float hcur = aces_gamut_cusp_table_0_hues_array[i];
    if (h > hcur)
    {
      i_lo = i;
    }
    else
    {
      i_hi = i;
    }
    i = (i_lo + i_hi) / 2;
  }
  vec3 lo = texture(aces_gamut_cusp_table_0Sampler, vec2((float(i_hi) - 1.0 + 0.5) / float(363), 0.5)).rgb;
  vec3 hi = texture(aces_gamut_cusp_table_0Sampler, vec2((float(i_hi) + 0.5) / float(363), 0.5)).rgb;
  float t = (h - aces_gamut_cusp_table_0_hues_array[i_hi - 1]) / (aces_gamut_cusp_table_0_hues_array[i_hi] - aces_gamut_cusp_table_0_hues_array[i_hi - 1]);
  return mix(lo, hi, t);
}
float aces_get_focus_gain0(float J, float cuspJ)
{
  float thr = mix(cuspJ, 208.257080, 0.300000);
  if (J > thr)
  {
    float gain = ( 208.25708 - thr) / max(0.0001, 208.25708 - J);
    gain = log(gain)/log(10.0);
    return gain * gain + 1.0;
  }
  else
  {
    return 1.0;
  }
}
float aces_solve_J_intersect0(float J, float M, float focusJ, float slope_gain)
{
  float M_scaled = M / slope_gain;
  float a = M_scaled / focusJ;
  if (J < focusJ)
  {
    float b = 1.0 - M_scaled;
    float c = -J;
    float det =  b * b - 4.f * a * c;
    float root =  sqrt(det);
    return -2.0 * c / (b + root);
  }
  else
  {
    float b = - (1.0 + M_scaled + 208.25708 * a);
    float c = 208.25708 * M_scaled + J;
    float det =  b * b - 4.f * a * c;
    float root =  sqrt(det);
    return -2.0 * c / (b - root);
  }
}
float aces_find_gamut_boundary_intersection0(vec2 JM_cusp, float gamma_top_inv, float gamma_bottom_inv, float J_intersect_source, float J_intersect_cusp, float slope)
{
  float M_boundary_lower = J_intersect_cusp * pow(J_intersect_source / J_intersect_cusp, gamma_bottom_inv) / (JM_cusp.r / JM_cusp.g - slope);
  float M_boundary_upper = JM_cusp.g * (208.25708 - J_intersect_cusp) * pow((208.25708 - J_intersect_source) / (208.25708 - J_intersect_cusp), gamma_top_inv) / (slope * JM_cusp.g + 208.25708 - JM_cusp.r);
  float smin = 0.0;
  {
    float a = M_boundary_lower;
    float b = M_boundary_upper;
    float s = 0.119999997 * JM_cusp.g;
    float h = max(s - abs(a - b), 0.0) / s;
    smin = min(a, b) - h * h * h * s * 0.16666666666666666;
  }
  return smin;
}
float aces_remap_M_fwd0(float M, float gamut_boundary_M, float reach_boundary_M)
{
  float boundary_ratio = gamut_boundary_M / reach_boundary_M;
  float proportion = max(boundary_ratio, 0.75);
  float threshold = proportion * gamut_boundary_M;
  if (proportion >= 1.0f || M <= threshold)
  {
    return M;
  }
  float m_offset = M - threshold;
  float gamut_offset = gamut_boundary_M - threshold;
  float reach_offset = reach_boundary_M - threshold;
  float scale = reach_offset / ((reach_offset / gamut_offset) - 1.0f);
  float nd = m_offset / scale;
  return threshold + scale * nd / (1.0f + nd);
}
vec3 aces_gamut_compress0(vec3 JMh, float Jx, vec3 JMGcusp, float reachMaxM)
{
  float J = JMh.r;
  float M = JMh.g;
  float h = JMh.b;
  if (M <= 0.0 || J > 208.25708)
  {
    return vec3(J, 0.0, h);
  }
  else
  {
    vec2 JMcusp = JMGcusp.rg;
    float focusJ = mix(JMcusp.r, 38.932209, min(1.0, 1.300000 - (JMcusp.r / 208.257080)));
    float slope_gain = 625.04541 * aces_get_focus_gain0(Jx, JMcusp.r);
    float J_intersect_source = aces_solve_J_intersect0(JMh.r, JMh.g, focusJ, slope_gain);
    float gamut_slope = (J_intersect_source < focusJ) ? J_intersect_source : (208.25708 - J_intersect_source);
    gamut_slope = gamut_slope * (J_intersect_source - focusJ) / (focusJ * slope_gain);
    float gamma_top_inv = JMGcusp.b;
    float gamma_bottom_inv = 0.841093898;
    float J_intersect_cusp = aces_solve_J_intersect0(JMcusp.r, JMcusp.g, focusJ, slope_gain);
    float gamutBoundaryM = aces_find_gamut_boundary_intersection0(JMcusp, gamma_top_inv, gamma_bottom_inv, J_intersect_source, J_intersect_cusp, gamut_slope);
    if (gamutBoundaryM <= 0.0)
    {
      return vec3(J, 0.0, h);
    }
    float reachBoundaryM = 208.25708 * pow(J_intersect_source / 208.25708,  0.879464149);
    reachBoundaryM = reachBoundaryM / ((208.25708 / reachMaxM) - gamut_slope);
    float remapped_M = aces_remap_M_fwd0(M, gamutBoundaryM, reachBoundaryM);
    float remapped_J = J_intersect_source + remapped_M * gamut_slope;
    return vec3(remapped_J, remapped_M, h);
  }
}

// Declaration of the OCIO shader function

vec4 antitone_render(vec4 inPixel)
{
  vec4 outColor = inPixel;
  
  // Add Range processing
  
  {
    outColor.rgb = max(vec3(0., 0., 0.), outColor.rgb);
    outColor.rgb = min(vec3(1024., 1024., 1024.), outColor.rgb);
  }
  
  // Add Matrix processing
  
  {
    vec4 res = vec4(outColor.rgb.r, outColor.rgb.g, outColor.rgb.b, outColor.a);
    vec4 tmp = res;
    res = mat4(0.69545224135745187, 0.044794563372037723, -0.0055258825581135443, 0., 0.14067869647029416, 0.85967111845642175, 0.0040252103059786595, 0., 0.16386906217225405, 0.0955343181715404, 1.0015006722521349, 0., 0., 0., 0., 1.) * tmp;
    outColor.rgb = vec3(res.x, res.y, res.z);
    outColor.a = res.w;
  }
  
  // Add FixedFunction 'ACES_OutputTransform20 (Forward)' processing
  
  {
    
    // Add RGB to JMh
    
    vec3 JMh;
    vec3 Aab;
    {
      {
        vec3 lms = mat3(0.445181042, 0.123734146, 0.0117007261, 0.34964928, 0.613643706, 0.0280607939, -0.00112973328, 0.0563228093, 0.753939092) * outColor.rgb;
        vec3 F_L_v = pow(abs(lms), vec3(0.419999987, 0.419999987, 0.419999987));
        vec3 rgb_a = (sign(lms) * F_L_v) / ( 27.1299992 + F_L_v);
        Aab = mat3(20.2588081, 15480., 1720., 10.1294041, -16887.2734, 1720., 0.506470203, 1407.27271, -3440.) * rgb_a.rgb;
      }
      {
        if (Aab.r <= 0.0)
        {
          JMh.rgb = vec3(0., 0., 0.);
        }
        else
        {
          float J = 100. * pow(Aab.r, 1.13705599);
          float M = (J == 0.0) ? 0.0 : sqrt(Aab.g * Aab.g + Aab.b * Aab.b);
          float h = (Aab.g == 0.0) ? 0.0 : atan(Aab.b, Aab.g) * 57.29577951308238;
          h = h - floor(h / 360.0) * 360.0;
          h = (h < 0.0) ? h + 360.0 : h;
          JMh.rgb = vec3(J, M, h);
        }
      }
      outColor.rgb = JMh;
    }
    float h_rad = outColor.b * 0.0174532924;
    float cos_hr = cos(h_rad);
    float sin_hr = sin(h_rad);
    
    // Add ToneScale and ChromaCompress (fwd)
    
    float J_ts = aces_tonescale_fwd0(outColor.r);
    // Sample tables (fwd)
    float reachMaxM = aces_reach_m_table_0_sample(outColor.b);
    
    {
      float J = outColor.r;
      float M = outColor.g;
      float h = outColor.b;
      float M_cp = M;
      if (M != 0.0)
      {
        float nJ = J_ts / 208.25708;
        float snJ = max(0.0, 1.0 - nJ);
        float Mnorm;
        {
          float cos_hr2 = 2.0 * cos_hr * cos_hr - 1.0;
          float sin_hr2 = 2.0 * cos_hr * sin_hr;
          float cos_hr3 = 4.0 * cos_hr * cos_hr * cos_hr - 3.0 * cos_hr;
          float sin_hr3 = 3.0 * sin_hr - 4.0 * sin_hr * sin_hr * sin_hr;
          vec3 cosines = vec3(cos_hr, cos_hr2, cos_hr3);
          vec3 cosine_weights = vec3(21.814416379852293, 31.678888572826388, 15.164865710067749);
          vec3 sines = vec3(sin_hr, sin_hr2, sin_hr3);
          vec3 sine_weights = vec3(28.207692783603669, -12.257307881011963, 17.68440548805237);
          Mnorm = dot(cosines, cosine_weights) + dot(sines, sine_weights) + 148.36123706298829;
        }
        float limit = pow(nJ, 0.879464149) * reachMaxM / Mnorm;
        M_cp = M * pow(J_ts / J, 0.879464149);
        M_cp = M_cp / Mnorm;
        M_cp = limit - aces_toe_fwd0(limit - M_cp, limit - 0.001, snJ * 0.67302388, sqrt(nJ * nJ + 0.00100000005));
        M_cp = aces_toe_fwd0(M_cp, limit, nJ * 7.93584251, snJ);
        M_cp = M_cp * Mnorm;
      }
      outColor.rgb = vec3(J_ts, M_cp, h);
    }
    
    // Add GamutCompress (fwd)
    
    {
      vec3 JMGcusp = aces_gamut_cusp_table_0_sample(outColor.b);
      outColor.rgb = aces_gamut_compress0(outColor.rgb, outColor.r, JMGcusp, reachMaxM);
    }
    
    // Add JMh to RGB
    
    {
      vec3 JMh = outColor.rgb;
      vec3 Aab;
      {
        Aab.r = pow(JMh.r * 0.00999999978, 0.879464149);
        Aab.g = JMh.g * cos_hr;
        Aab.b = JMh.g * sin_hr;
      }
      {
        vec3 rgb_a = mat3(0.0323680304, 0.0323680304, 0.0323680304, 2.07657631e-05, -4.10250432e-05, -1.01296409e-05, 1.3260621e-05, -1.20174373e-05, -0.000290076074) * Aab.rgb;
        vec3 rgb_a_lim = min( abs(rgb_a), vec3(0.99000001, 0.99000001, 0.99000001) );
        vec3 lms = sign(rgb_a) * pow( 27.1299992 * rgb_a_lim / (1.0f - rgb_a_lim), vec3(2.38095236, 2.38095236, 2.38095236));
        JMh.rgb = mat3(4.18920946, -0.841454685, 0.00181781233, -2.83307672, 2.44025636, -0.111337893, -0.0962110385, -0.338880897, 1.36944115) * lms;
      }
      outColor.rgb = JMh;
    }
  }
  
  // Add Range processing
  
  {
    outColor.rgb = max(vec3(0., 0., 0.), outColor.rgb);
    outColor.rgb = min(vec3(5., 5., 5.), outColor.rgb);
  }

  return outColor;
}
